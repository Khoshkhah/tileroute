//! `tileroute check <db.duckdb> [--mode driving] [--finest 20] [--pairs N] [--pairs-file CSV] [--no-reference] [--write-pairs FILE]`:
//! make the shortcuts, then compare routes with duckOSM's router (all pairs, or N random pairs).
//! `tileroute build <db.duckdb> -o <folder>`: make the shortcuts and write the tile files.
//! `tileroute route <folder> --pairs-file CSV`: answer the pairs from the tile files only.

use rayon::prelude::*;
use std::time::Instant;
use tileroute::{disk, hier, load};

/// The memory of this process now (resident set), from `ps`.
fn rss() -> String {
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<f64>().ok())
        .map_or("unknown".into(), |kb| format!("{:.0} MB", kb / 1024.0))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("build") => return build(&args),
        Some("route") => return route_files(&args),
        Some("slim") => return slim(&args),
        Some("points") => return points(&args),
        Some("weigh") => return weigh(&args),
        Some("reference") => return reference(&args),
        _ => {}
    }
    let opt = |name: &str, default: &str| -> String {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or(default.into())
    };
    let (Some("check"), Some(db)) = (args.first().map(String::as_str), args.get(1)) else {
        return Err("usage: tileroute check <db.duckdb> [--mode driving] [--finest 20] [--pairs N]".into());
    };
    let mode = opt("--mode", "driving");
    let finest: u8 = opt("--finest", "20").parse().map_err(|e| format!("--finest: {e}"))?;

    let t = Instant::now();
    // --partition LEAF: the cells from the inertial flow partition (step D1), not from the tiles
    let leaf = args.iter().position(|a| a == "--partition").map(|i| args.get(i + 1).and_then(|x| x.parse().ok()).unwrap_or(16usize));
    let net = load::read_with(db.as_ref(), &mode, finest, leaf)?;
    let n = net.graph.n_nodes();
    println!("read: edge graph with {n} nodes (road edges) and {} arcs (edge_graph rows), {:.2} s", net.turns.len(), t.elapsed().as_secs_f64());

    let t = Instant::now();
    let core: u8 = opt("--core", "0").parse().map_err(|e| format!("--core: {e}"))?;
    let renumber = !args.iter().any(|a| a == "--no-renumber");
    // --top-ch Z: the tile steps down to zoom Z + 1, then a classic contraction hierarchy for the top (zoom Z and coarser)
    let top_ch: u8 = opt("--top-ch", "0").parse().map_err(|e| format!("--top-ch: {e}"))?;
    let (core, core_ch) = if top_ch > 0 { (top_ch, true) } else { (core, false) };
    let t_prep = Instant::now();
    let h = hier::prepare(&net.turns, net.level.clone(), net.cell.clone(), core, renumber, core_ch);
    if core_ch {
        println!("top: classic contraction hierarchy for zoom {top_ch} and coarser; preparation {:.2} s", t_prep.elapsed().as_secs_f64());
    }
    println!("shortcuts: {} pairs made, {} arcs in all, {:.3} s", h.shortcuts.iter().sum::<usize>(), h.n_arcs(), t.elapsed().as_secs_f64());
    for (z, c) in h.shortcuts.iter().enumerate() {
        let nodes = net.level.iter().filter(|&&l| l as usize == z).count();
        if *c > 0 || nodes > 0 {
            let (k, sum, max) = h.steps[z];
            println!(
                "  zoom {z:2}: {nodes:8} nodes, {c:8} shortcuts, {k:7} sets, steps {:9.3} ms in all, mean {:8.4} ms, largest {:8.3} ms",
                sum * 1e3,
                if k > 0 { sum / k as f64 * 1e3 } else { 0.0 },
                max * 1e3
            );
        }
    }

    println!("witness search: {} of {} shortcut pairs not needed", h.witnessed.iter().sum::<usize>(), h.shortcuts.iter().sum::<usize>());
    if args.iter().any(|a| a == "--degrees") {
        let d = h.degrees();
        let (nn, na) = d.iter().fold((0, 0), |a, x| (a.0 + x.0, a.1 + x.1));
        println!("degree: mean {:.2} arcs out of each node ({nn} nodes, {na} arcs)", na as f64 / nn as f64);
        for (z, &(k, all, up, max)) in d.iter().enumerate() {
            if k > 0 {
                println!("  zoom {z:2}: {k:8} nodes, mean out-degree {:7.1}, mean forward-search degree {:7.1}, largest {max}", all as f64 / k as f64, up as f64 / k as f64);
            }
        }
    }
    let (k, sum) = h.inner.iter().fold((0u64, 0u64), |a, x| (a.0 + x.0, a.1 + x.1));
    println!("inner nodes of a shortcut path: mean {:.2} over {k} pairs, largest {}", sum as f64 / k.max(1) as f64, h.inner.iter().map(|x| x.2).max().unwrap_or(0));
    for (z, &(k, sum, max)) in h.inner.iter().enumerate() {
        if k > 0 {
            println!("  zoom {z:2}: mean {:.2}, largest {max}", sum as f64 / k as f64);
        }
    }
    // what a query server keeps: the hierarchy and the edge ids (8 bytes and about 16 bytes of index for each node)
    let ids_bytes = n * 8 + n * 16;
    println!(
        "query data: hierarchy {:.0} MB + edge ids {:.0} MB = {:.0} MB; process now {} (after the preparation)",
        h.query_bytes() as f64 / 1e6,
        ids_bytes as f64 / 1e6,
        (h.query_bytes() + ids_bytes) as f64 / 1e6,
        rss()
    );
    let limit: Option<u8> = args.iter().position(|a| a == "--limit").and_then(|i| args.get(i + 1)).and_then(|x| x.parse().ok());
    let route = |s: u32, t: u32| match limit {
        Some(k) => h.route_limited(s, t, k),
        None => h.route(s, t),
    };

    // the pairs: all, or N random ones (a fixed sequence)
    let from_file = args.iter().position(|a| a == "--pairs-file").and_then(|i| args.get(i + 1));
    let pairs: Vec<(u32, u32)> = if let Some(path) = from_file {
        // a CSV with the duckOSM edge ids of the start and the target in its first two columns, and a header line
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        let node = |id: &str| -> Result<u32, String> {
            let id: i64 = id.trim().parse().map_err(|e| format!("{path}: {e}"))?;
            net.graph.index_of(id).map(|i| i as u32).ok_or(format!("{path}: edge {id} is not in the graph"))
        };
        text.lines().skip(1).filter(|l| !l.is_empty()).map(|l| {
            let mut f = l.split(',');
            Ok((node(f.next().unwrap_or(""))?, node(f.next().unwrap_or(""))?))
        }).collect::<Result<_, String>>()?
    } else { match opt("--pairs", "").parse::<usize>() {
        Ok(k) => {
            let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
            let mut next = || {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x % n as u64) as u32
            };
            (0..k).map(|_| (next(), next())).collect()
        }
        Err(_) => (0..n as u32).flat_map(|s| (0..n as u32).map(move |t| (s, t))).collect(),
    } };
    let weight: std::collections::HashMap<(u32, u32), f64> = net.turns.iter().map(|&(u, w, c)| ((u, w), c)).collect();

    // each query is timed alone; the mean is the time of one query on one thread
    let timed = |f: &(dyn Fn(u32, u32) -> f64 + Sync)| -> f64 {
        pairs.par_iter().map(|&(s, t)| f(s, t)).sum::<f64>() / pairs.len().max(1) as f64 * 1e3
    };
    let ours: Vec<Option<(f64, Vec<u32>)>> = pairs.par_iter().map(|&(s, t)| route(s, t)).collect();
    let ms_ours = timed(&|s, t| {
        let t0 = Instant::now();
        std::hint::black_box(route(s, t));
        t0.elapsed().as_secs_f64()
    });
    // the tiles (cells) that one query settles, on average
    let mean = |f: &(dyn Fn(u32, u32) -> usize + Sync)| pairs.par_iter().map(|&(s, t)| f(s, t)).sum::<usize>() as f64 / pairs.len().max(1) as f64;
    println!("tiles per query: {:.1}", mean(&|s, t| h.route_stats(s, t).1));
    if args.iter().any(|a| a == "--profile") {
        let by = h.arcs_by_level(&pairs);
        let all: usize = by.iter().sum();
        let line: Vec<String> = by.iter().enumerate().filter(|(_, &x)| x > 0).map(|(z, &x)| format!("{z}:{:.0}%", x as f64 * 100.0 / all as f64)).collect();
        println!("arcs opened by the level of the settled node: {}", line.join(" "));
        let rows: Vec<(usize, f64, f64, usize, usize)> = pairs.iter().map(|&(s, t)| h.profile(s, t)).collect();
        let k = rows.len().max(1) as f64;
        let sum = |f: &dyn Fn(&(usize, f64, f64, usize, usize)) -> f64| rows.iter().map(f).sum::<f64>() / k;
        println!(
            "profile (one thread): {:.0} nodes settled, {:.0} arcs opened, search {:.3} ms, unpack {:.3} ms, {:.0} nodes on the path",
            sum(&|r| r.0 as f64),
            sum(&|r| r.4 as f64),
            sum(&|r| r.1) * 1e3,
            sum(&|r| r.2) * 1e3,
            sum(&|r| r.3 as f64)
        );
    }
    if args.iter().any(|a| a == "--cch") {
        let files = args.iter().position(|a| a == "--cch-files").and_then(|i| args.get(i + 1));
        cch_check(&net, &pairs, &ours, files.map(|s| s.as_ref()))?;
    }
    if args.iter().any(|a| a == "--weights") {
        weights_check(&net, db.as_ref(), &mode, &pairs)?;
    }
    let ids = &net.graph.ids;
    if let Some(dir) = args.iter().position(|a| a == "--files").and_then(|i| args.get(i + 1)) {
        // the routes from the tile files must equal the routes in memory: the same cost, and a path of that cost
        let store = disk::Store::open(dir.as_ref(), 64).map_err(|e| e.to_string())?;
        let from_files: Vec<Option<(f64, Vec<i64>)>> =
            pairs.par_iter().map(|&(s, t)| store.route(ids[s as usize], ids[t as usize]).map(|r| r.0)).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        let index: std::collections::HashMap<i64, u32> = ids.iter().enumerate().map(|(i, &x)| (x, i as u32)).collect();
        let (mut same, mut tie, mut wrong) = (0usize, 0usize, 0usize);
        for (f, o) in from_files.iter().zip(&ours) {
            match (f, o) {
                (None, None) => same += 1,
                (Some((d, p)), Some((od, op))) if d == od => {
                    let own = p.windows(2).map(|w| weight.get(&(index[&w[0]], index[&w[1]])).copied()).try_fold(0.0, |s, c| c.map(|c| s + c));
                    let mine: Vec<i64> = op.iter().map(|&v| ids[v as usize]).collect();
                    if own != Some(*d) {
                        wrong += 1;
                    } else if *p == mine {
                        same += 1;
                    } else {
                        tie += 1;
                    }
                }
                _ => wrong += 1,
            }
        }
        println!("tile files against memory: {} pairs | same {same} | tie (other path, equal cost) {tie} | wrong {wrong}", pairs.len());
        if wrong > 0 {
            return Err(format!("{wrong} wrong routes from the tile files"));
        }
    }
    if let Some(path) = args.iter().position(|a| a == "--write-pairs").and_then(|i| args.get(i + 1)) {
        // source,target,cost (the sum of the arc costs; -1: no path), for other routers
        let mut out = String::from("source,target,cost\n");
        for (&(s, t), o) in pairs.iter().zip(&ours) {
            out += &format!("{},{},{}\n", ids[s as usize], ids[t as usize], o.as_ref().map_or(-1.0, |x| x.0));
        }
        std::fs::write(path, out).map_err(|e| format!("{path}: {e}"))?;
    }
    if args.iter().any(|a| a == "--no-reference") {
        println!("pairs: {} | tileroute {ms_ours:.3} ms per query (one thread)", pairs.len());
        return Ok(());
    }
    let refs: Vec<Option<(f64, Vec<i64>)>> =
        pairs.par_iter().map(|&(s, t)| net.graph.bidirectional_dijkstra(ids[s as usize], ids[t as usize]).expect("in G")).collect();
    let ms_ref = timed(&|s, t| {
        let t0 = Instant::now();
        std::hint::black_box(net.graph.bidirectional_dijkstra(ids[s as usize], ids[t as usize]).ok());
        t0.elapsed().as_secs_f64()
    });

    let (mut same, mut tie, mut none, mut bad) = (0usize, 0usize, 0usize, Vec::new());
    for (i, (o, r)) in ours.iter().zip(&refs).enumerate() {
        match (o, r) {
            (None, None) => none += 1,
            (Some((d, p)), Some((rd, rp))) => {
                let mine: Vec<i64> = p.iter().map(|&v| ids[v as usize]).collect();
                // the cost of our path, from the arcs, in order
                let own = p.windows(2).map(|w| weight.get(&(w[0], w[1])).copied()).try_fold(0.0, |s, c| c.map(|c| s + c));
                if own != Some(*d) || d != rd {
                    bad.push(i);
                } else if &mine == rp {
                    same += 1;
                } else {
                    tie += 1;
                }
            }
            _ => bad.push(i),
        }
    }
    if limit.is_some() {
        // how much longer the wrong routes are
        let extra: Vec<f64> = bad.iter().filter_map(|&i| match (&ours[i], &refs[i]) {
            (Some((d, _)), Some((rd, _))) if *rd > 0.0 => Some(d / rd - 1.0),
            _ => None,
        }).collect();
        let lost = bad.iter().filter(|&&i| ours[i].is_none() && refs[i].is_some()).count();
        let mean = extra.iter().sum::<f64>() / extra.len().max(1) as f64;
        let max = extra.iter().cloned().fold(0.0, f64::max);
        println!("limit: {} routes longer (mean +{:.2}%, largest +{:.1}%), {lost} routes not found", extra.len(), mean * 100.0, max * 100.0);
        println!("time per query (one thread): tileroute {ms_ours:.3} ms");
        return Ok(());
    }
    println!(
        "pairs: {} | same path {same} | tie (other path, equal cost) {tie} | no path {none} | wrong {}",
        pairs.len(),
        bad.len()
    );
    println!("time per query (one thread): tileroute {ms_ours:.3} ms, duckOSM router {ms_ref:.3} ms");
    for &i in bad.iter().take(5) {
        let (s, t) = pairs[i];
        println!("  wrong: {} -> {}: ours {:?}, duckOSM {:?}", ids[s as usize], ids[t as usize], ours[i].as_ref().map(|x| x.0), refs[i].as_ref().map(|x| x.0));
    }
    if bad.is_empty() { Ok(()) } else { Err(format!("{} wrong routes", bad.len())) }
}

fn arg<'a>(args: &'a [String], name: &str) -> Option<&'a String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1))
}

/// `tileroute build <db.duckdb> -o <folder> [--mode driving] [--finest 20]`
fn build(args: &[String]) -> Result<(), String> {
    let usage = "usage: tileroute build <db.duckdb> -o <folder> [--mode driving] [--finest 20] [--by-tile [--partition LEAF] [--top Z]]";
    let (Some(db), Some(dir)) = (args.get(1), arg(args, "-o")) else { return Err(usage.into()) };
    let mode = arg(args, "--mode").map_or("driving", |s| s.as_str());
    let finest: u8 = arg(args, "--finest").map_or(Ok(20), |s| s.parse()).map_err(|e| format!("--finest: {e}"))?;
    if args.iter().any(|a| a == "--by-tile") {
        // --partition LEAF: the cells from the partition; then the files are cells of zoom --top + 1 (default 3)
        let leaf: Option<usize> = arg(args, "--partition").map(|s| s.parse()).transpose().map_err(|e| format!("--partition: {e}"))?;
        let top: u8 = arg(args, "--top").map_or(Ok(if leaf.is_some() { 2 } else { disk::TOP }), |s| s.parse()).map_err(|e| format!("--top: {e}"))?;
        let xyz = args.iter().any(|a| a == "--xyz-files");
        if args.iter().any(|a| a == "--cch") {
            // the structure with no costs and the costs of each metric, one unit at a time
            let leaf = leaf.ok_or("--by-tile --cch needs --partition")?;
            let names: Vec<String> = arg(args, "--metrics").map_or("time", |s| s.as_str()).split(',').map(String::from).collect();
            let middle = args.iter().any(|a| a == "--middle-nodes");
            let r = tileroute::cchbuild::build(db.as_ref(), mode, finest, top, leaf, xyz, middle, &names, dir.as_ref())?;
            println!(
                "built unit by unit: {} nodes ({} in the top), {} arcs, {} edges, {} files, {:.1} MB",
                r.nodes, r.top_nodes, r.arcs, r.edges, r.files, r.bytes as f64 / 1e6
            );
            for (what, secs) in &r.times {
                println!("  {what}: {secs:.2} s");
            }
            println!("peak memory of this process: {:.0} MB", disk::peak_rss() as f64 / 1e6);
            return Ok(());
        }
        let r = tileroute::bytile::build(db.as_ref(), mode, finest, top, leaf, xyz, dir.as_ref())?;
        println!(
            "built tile by tile: {} nodes ({} in the top), {} arcs, {} files of zoom {}, {:.1} MB",
            r.nodes, r.top_nodes, r.arcs, r.tiles, if xyz { 8 } else { top + 1 }, r.bytes as f64 / 1e6
        );
        for (what, secs) in &r.times {
            println!("  {what}: {secs:.2} s");
        }
        println!("peak memory of this process: {:.0} MB", disk::peak_rss() as f64 / 1e6);
        return Ok(());
    }
    let t = Instant::now();
    // --partition LEAF: the order from the partition; --xyz-files: then the files are zoom-8 XYZ tiles (else cells of
    // zoom --top + 1)
    let leaf: Option<usize> = arg(args, "--partition").map(|s| s.parse()).transpose().map_err(|e| format!("--partition: {e}"))?;
    let top: u8 = arg(args, "--top").map_or(Ok(if leaf.is_some() { 2 } else { disk::TOP }), |s| s.parse()).map_err(|e| format!("--top: {e}"))?;
    let xyz = args.iter().any(|a| a == "--xyz-files");
    let net = load::read_with(db.as_ref(), mode, finest, leaf)?;
    println!("read: {} nodes, {} arcs, {:.2} s", net.graph.n_nodes(), net.turns.len(), t.elapsed().as_secs_f64());
    if args.iter().any(|a| a == "--cch") {
        return build_cch(args, &net, db.as_ref(), mode, top, xyz, dir.as_ref());
    }
    let t = Instant::now();
    let h = hier::prepare(&net.turns, net.level.clone(), net.cell.clone(), 0, true, false);
    println!("shortcuts: {} arcs in all, {:.2} s", h.n_arcs(), t.elapsed().as_secs_f64());
    let t = Instant::now();
    let w = h.write_files_by(&net.graph.ids, dir.as_ref(), top, xyz.then_some(&net.home[..])).map_err(|e| e.to_string())?;
    let tiles: Vec<&(String, u32, u64)> = w.iter().filter(|x| x.0.ends_with(".trt") && x.0 != "top.trt").collect();
    let size = |name: &str| w.iter().find(|x| x.0 == name).map_or(0, |x| x.2);
    let mb = |b: u64| b as f64 / 1e6;
    println!("wrote {dir}: {} files, {:.1} MB in all, {:.2} s", w.len(), mb(w.iter().map(|x| x.2).sum()), t.elapsed().as_secs_f64());
    println!("  top.trt: {} nodes, {:.1} MB", w.iter().find(|x| x.0 == "top.trt").map_or(0, |x| x.1), mb(size("top.trt")));
    if let (Some(big), k) = (tiles.iter().max_by_key(|x| x.2), tiles.len()) {
        let sum: u64 = tiles.iter().map(|x| x.2).sum();
        println!("  {k} tile files: {:.1} MB in all, mean {:.1} MB, largest {} {:.1} MB ({} nodes)", mb(sum), mb(sum) / k as f64, big.0, mb(big.2), big.1);
    }
    println!("  edges.idx: {:.1} MB, index.bin: {} bytes", mb(size("edges.idx")), size("index.bin"));
    Ok(())
}

/// `tileroute slim <db> <out.duckdb> [--mode driving]`: copy only what tileroute reads (the arcs of the mode, the road
/// edges and the points of their ends) into a new, small database.
fn slim(args: &[String]) -> Result<(), String> {
    let (Some(db), Some(out)) = (args.get(1), args.get(2)) else { return Err("usage: tileroute slim <db> <out.duckdb> [--mode driving]".into()) };
    let mode = arg(args, "--mode").map_or("driving", |s| s.as_str());
    // a writable connection: the source read-only, the new file writable
    let con = duckdb::Connection::open_in_memory().map_err(|e| e.to_string())?;
    con.execute_batch(&format!("LOAD spatial; ATTACH '{db}' AS src (READ_ONLY); USE src;")).map_err(|e| e.to_string())?;
    let relation = load::relation(&con, mode)?;
    let sql = format!(
        "ATTACH '{out}' AS slim;
         CREATE SCHEMA slim.{mode};
         CREATE TABLE slim.{mode}.edge_graph AS SELECT from_edge, to_edge, cost FROM {relation};
         CREATE TABLE slim.{mode}.edges AS SELECT edge_id, source, target, length_m FROM {mode}.edges;
         CREATE TABLE slim.{mode}.nodes AS SELECT node_id, geom FROM {mode}.nodes;
         DETACH slim;"
    );
    con.execute_batch(&sql).map_err(|e| e.to_string())?;
    println!("wrote {out}: {mode}.edge_graph, {mode}.edges, {mode}.nodes");
    Ok(())
}

/// `tileroute points <db> <pairs.csv> <out.csv> [--mode driving]`: for each pair (source, target, cost), the point where
/// the source road edge starts and the point where the target road edge ends, and the cost of the target edge (for
/// other routers, which take points).
fn points(args: &[String]) -> Result<(), String> {
    let (Some(db), Some(pairs), Some(out)) = (args.get(1), args.get(2), args.get(3)) else {
        return Err("usage: tileroute points <db> <pairs.csv> <out.csv> [--mode driving]".into());
    };
    let mode = arg(args, "--mode").map_or("driving", |s| s.as_str());
    let con = load::open(db.as_ref())?;
    con.execute_batch(&format!(
        "LOAD spatial;
         COPY (SELECT p.source, p.target, p.cost,
                      ST_X(a.geom) AS lon1, ST_Y(a.geom) AS lat1, ST_X(b.geom) AS lon2, ST_Y(b.geom) AS lat2,
                      (SELECT min(g.cost)::DOUBLE FROM {mode}.edge_graph g WHERE g.from_edge = p.target) AS target_cost
               FROM read_csv('{pairs}') p
               JOIN {mode}.edges e1 ON e1.edge_id = p.source JOIN {mode}.nodes a ON a.node_id = e1.source
               JOIN {mode}.edges e2 ON e2.edge_id = p.target JOIN {mode}.nodes b ON b.node_id = e2.target)
         TO '{out}' (HEADER);"
    ))
    .map_err(|e| e.to_string())?;
    println!("wrote {out}");
    Ok(())
}

/// The memory lines of this process from /proc (Linux only).
fn proc_memory() -> String {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    s.lines().filter(|l| ["VmHWM", "RssAnon", "RssFile"].iter().any(|k| l.starts_with(k))).map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join(", ")
}

/// `tileroute route <folder> --pairs-file CSV [--cache 64]`: a process that only reads the tile files.
fn route_files(args: &[String]) -> Result<(), String> {
    let usage = "usage: tileroute route <folder> (--pairs-file CSV | --random N) [--cache 64]";
    let Some(dir) = args.get(1) else { return Err(usage.into()) };
    let cache: usize = arg(args, "--cache").map_or(Ok(64), |s| s.parse()).map_err(|e| format!("--cache: {e}"))?;
    // --metric NAME: a folder of `build --cch` (the shared structure and the costs of this metric)
    let cch = match arg(args, "--metric") {
        Some(m) => Some(tileroute::cchdisk::Store::open(dir.as_ref(), m).map_err(|e| e.to_string())?),
        None => None,
    };
    let store = if cch.is_some() { None } else { Some(disk::Store::open(dir.as_ref(), cache).map_err(|e| e.to_string())?) };
    let pairs: Vec<(i64, i64)> = if let Some(k) = arg(args, "--random") {
        // N random pairs of edge ids from edges.idx (a fixed sequence), for an area with no pairs file
        let k: usize = k.parse().map_err(|e| format!("--random: {e}"))?;
        let b = std::fs::read(std::path::Path::new(dir).join("edges.idx")).map_err(|e| e.to_string())?;
        let n = u64::from_le_bytes(b[8..16].try_into().expect("8 bytes")) as usize;
        let id = |i: usize| i64::from_le_bytes(b[16 + 8 * i..24 + 8 * i].try_into().expect("8 bytes"));
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n as u64) as usize
        };
        (0..k).map(|_| (id(next()), id(next()))).collect()
    } else {
        let Some(path) = arg(args, "--pairs-file") else { return Err(usage.into()) };
        // a CSV with the duckOSM edge ids of the start and the target in its first two columns, and a header line
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        text
        .lines()
        .skip(1)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let mut f = l.split(',').map(|x| x.trim().parse::<i64>());
            match (f.next(), f.next()) {
                (Some(Ok(a)), Some(Ok(b))) => Ok((a, b)),
                _ => Err(format!("{path}: not two edge ids: {l}")),
            }
        })
        .collect::<Result<_, _>>()?
    };
    let n_files = cch.as_ref().map_or_else(|| store.as_ref().map_or(0, |s| s.n_files()), |c| c.n_files());
    println!("opened {dir}: {n_files} tile files; process {:.1} MB peak", disk::peak_rss() as f64 / 1e6);
    let route = |a: i64, b: i64| -> Result<(Option<(f64, Vec<i64>)>, disk::Stats), String> {
        match (&cch, &store) {
            (Some(c), _) => c.route(a, b).map(|(r, tiles)| (r, disk::Stats { tiles, bytes: 0 })).map_err(|e| e.to_string()),
            (None, Some(s)) => s.route(a, b).map_err(|e| e.to_string()),
            _ => unreachable!("one store"),
        }
    };
    // --write CSV: the cost of each pair (the sum of the arc costs; -1: no path), from the first pass
    let mut write: Option<String> = arg(args, "--write").map(|_| String::from("source,target,cost\n"));
    // two passes on one thread: the first maps the files, the second finds them in the cache of the operating system
    for pass in ["first", "second"] {
        let (mut ms, mut tiles, mut bytes, mut max_t, mut max_b, mut found) = (0.0, 0usize, 0usize, 0usize, 0usize, 0usize);
        for &(a, b) in &pairs {
            let t = Instant::now();
            let (r, st) = route(a, b)?;
            ms += t.elapsed().as_secs_f64() * 1e3;
            if pass == "first" {
                if let Some(w) = write.as_mut() {
                    w.push_str(&format!("{a},{b},{}\n", r.as_ref().map_or(-1.0, |x| x.0)));
                }
            }
            found += r.is_some() as usize;
            tiles += st.tiles;
            bytes += st.bytes;
            max_t = max_t.max(st.tiles);
            max_b = max_b.max(st.bytes);
        }
        let k = pairs.len().max(1) as f64;
        println!(
            "{pass} pass: {} pairs, {found} routes | {:.3} ms per query | tiles read: mean {:.1}, largest {max_t} | bytes read: mean {:.0} KB, largest {:.0} KB",
            pairs.len(),
            ms / k,
            tiles as f64 / k,
            bytes as f64 / k / 1e3,
            max_b as f64 / 1e3
        );
    }
    if let (Some(path), Some(w)) = (arg(args, "--write"), write) {
        std::fs::write(path, w).map_err(|e| format!("{path}: {e}"))?;
    }
    let mapped = store.as_ref().map_or(n_files as u64, |s| s.mapped.load(std::sync::atomic::Ordering::Relaxed));
    println!("files mapped: {mapped}; peak memory of this process: {:.1} MB ({})", disk::peak_rss() as f64 / 1e6, proc_memory());
    Ok(())
}

/// `check --cch`: the structure with no costs and its customization (`tileroute::cch`), against the routes of the
/// hierarchy: the same cost for each pair, and a path of that cost.
fn cch_check(net: &load::Network, pairs: &[(u32, u32)], ours: &[Option<(f64, Vec<u32>)>], files: Option<&std::path::Path>) -> Result<(), String> {
    use tileroute::cch::Cch;
    let n = net.graph.n_nodes();
    let t = Instant::now();
    let c = Cch::new(n, net.turns.iter().map(|a| (a.0, a.1)), &net.level, &net.cell);
    let t_struct = t.elapsed().as_secs_f64();
    let m = c.customize(&net.turns, !std::env::args().any(|a| a == "--no-reduce"));
    println!(
        "cch: structure {:.2} s ({} edges, elimination tree depth {}, {:.0} MB); customization: triangles {:.2} s, arc reduction and mask {:.2} s ({} -> {} search arcs, {:.0} MB for this metric)",
        t_struct,
        c.n_edges(),
        c.max_depth(),
        c.bytes() as f64 / 1e6,
        m.times.0,
        m.times.1,
        m.before,
        m.n_search_arcs(),
        m.bytes() as f64 / 1e6
    );
    let weight: std::collections::HashMap<(u32, u32), f64> = net.turns.iter().map(|a| ((a.0, a.1), a.2)).collect();
    let (mut same, mut tie, mut wrong) = (0usize, 0usize, 0usize);
    let (mut settled, mut relaxed) = (0usize, 0usize);
    for (&(s, t), o) in pairs.iter().zip(ours) {
        let (r, st) = m.route(s, t);
        settled += st.settled;
        relaxed += st.relaxed;
        match (&r, o) {
            (None, None) => same += 1,
            (Some((d, p)), Some((od, op))) if d == od => {
                let own = p.windows(2).map(|w| weight.get(&(w[0], w[1])).copied()).try_fold(0.0, |s, c| c.map(|c| s + c));
                if own != Some(*d) {
                    wrong += 1;
                } else if p == op {
                    same += 1;
                } else {
                    tie += 1;
                }
            }
            _ => wrong += 1,
        }
    }
    let k = pairs.len().max(1);
    let t = Instant::now();
    for &(s, t) in pairs {
        std::hint::black_box(m.route(s, t));
    }
    println!(
        "cch against the hierarchy: {} pairs | same {same} | tie {tie} | wrong {wrong} | {:.3} ms per query (one thread), {} settled, {} arcs opened",
        pairs.len(),
        t.elapsed().as_secs_f64() / k as f64 * 1e3,
        settled / k,
        relaxed / k
    );
    if let Some(dir) = files {
        // the routes from the files (metric "time") must equal the routes of the metric in memory
        let store = tileroute::cchdisk::Store::open(dir, "time").map_err(|e| e.to_string())?;
        let ids = &net.graph.ids;
        let index: std::collections::HashMap<i64, u32> = ids.iter().enumerate().map(|(i, &x)| (x, i as u32)).collect();
        let (mut same, mut tie, mut wrong) = (0usize, 0usize, 0usize);
        for &(s, t) in pairs {
            let mem = m.route(s, t).0.map(|(c, p)| (c, p.iter().map(|&v| ids[v as usize]).collect::<Vec<i64>>()));
            let (disk, _) = store.route(ids[s as usize], ids[t as usize]).map_err(|e| e.to_string())?;
            match (&disk, &mem) {
                (None, None) => same += 1,
                (Some((d, p)), Some((md, mp))) if d == md => {
                    // the path of the files must cost what it says
                    let own = p.windows(2).map(|w| weight.get(&(index[&w[0]], index[&w[1]])).copied()).try_fold(0.0, |s, c| c.map(|c| s + c));
                    if own != Some(*d) {
                        wrong += 1;
                    } else if p == mp {
                        same += 1;
                    } else {
                        tie += 1;
                    }
                }
                _ => wrong += 1,
            }
        }
        println!("cch files against memory: {} pairs | same {same} | tie {tie} | wrong {wrong}", pairs.len());
        if wrong > 0 {
            return Err(format!("{wrong} routes from the cch files differ from memory"));
        }
    }
    Ok(())
}

/// `check --weights`: is the best of the routes of single costs (time, length) a good estimate of the best route for a
/// mix of them? For each mix: cost = l * time / mean time + (1 - l) * length / mean length, with the exact route from
/// a customization of that mix.
fn weights_check(net: &load::Network, db: &std::path::Path, mode: &str, pairs: &[(u32, u32)]) -> Result<(), String> {
    use tileroute::cch::Cch;
    let time = net.turns.clone();
    let length = arc_lengths(net, db, mode)?;
    let k = time.len() as f64;
    let (mt, ml) = (time.iter().map(|a| a.2).sum::<f64>() / k, length.iter().map(|a| a.2).sum::<f64>() / k);
    let mix = |l: f64| -> Vec<(u32, u32, f64)> { time.iter().zip(&length).map(|(a, b)| (a.0, a.1, l * a.2 / mt + (1.0 - l) * b.2 / ml)).collect() };
    let weight: std::collections::HashMap<(u32, u32), (f64, f64)> = time.iter().zip(&length).map(|(a, b)| ((a.0, a.1), (a.2, b.2))).collect();
    let c = Cch::new(net.graph.n_nodes(), time.iter().map(|a| (a.0, a.1)), &net.level, &net.cell);
    let t = Instant::now();
    let candidates = [c.customize(&time, true), c.customize(&length, true), c.customize(&mix(0.5), true)];
    println!("weights: 3 candidate metrics (time, length, 50/50 mix) customized in {:.1} s", t.elapsed().as_secs_f64());
    let routes: Vec<Vec<Option<Vec<u32>>>> = candidates.iter().map(|m| pairs.par_iter().map(|&(s, t)| m.route(s, t).0.map(|r| r.1)).collect()).collect();
    println!("{:>5} | {:>28} | {:>28}", "l", "best of time, length", "best of time, length, 50/50");
    println!("{:>5} | {:>8} {:>9} {:>9} | {:>8} {:>9} {:>9}", "", "exact", "mean err", "max err", "exact", "mean err", "max err");
    for l in [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9] {
        let m = c.customize(&mix(l), true);
        let cost_of = |p: &Vec<u32>| p.windows(2).map(|w| { let (a, b) = weight[&(w[0], w[1])]; l * a / mt + (1.0 - l) * b / ml }).sum::<f64>();
        let mut out = String::new();
        for kk in [2usize, 3] {
            let (mut n, mut exact, mut sum, mut max) = (0usize, 0usize, 0.0f64, 0.0f64);
            for (i, &(s, t)) in pairs.iter().enumerate() {
                let Some((best, _)) = m.route(s, t).0 else { continue };
                let est = routes[..kk].iter().filter_map(|r| r[i].as_ref()).map(cost_of).fold(f64::INFINITY, f64::min);
                if best <= 0.0 {
                    continue;
                }
                let err = (est - best) / best;
                n += 1;
                if err <= 1e-9 {
                    exact += 1;
                }
                sum += err.max(0.0);
                max = max.max(err);
            }
            out += &format!(" | {:>7.1}% {:>8.3}% {:>8.2}%", exact as f64 * 100.0 / n.max(1) as f64, sum * 100.0 / n.max(1) as f64, max * 100.0);
        }
        println!("{l:>5.1}{out}");
    }
    Ok(())
}

/// The time and the length of each node (its road edge): the cost of its arcs (duckOSM's cost is the time of the first
/// road edge) and `length_m`.
fn node_items(net: &load::Network, db: &std::path::Path, mode: &str) -> Result<Vec<[f32; 2]>, String> {
    let mut items = vec![[0.0f32; 2]; net.graph.n_nodes()];
    for &(u, _, c) in &net.turns {
        items[u as usize][0] = c as f32;
    }
    for (u, _, l) in arc_lengths(net, db, mode)? {
        items[u as usize][1] = l as f32;
    }
    Ok(items)
}

/// The length of each arc of `net.turns`: the length of its first road edge (duckOSM's cost is the time of the first
/// road edge).
fn arc_lengths(net: &load::Network, db: &std::path::Path, mode: &str) -> Result<Vec<(u32, u32, f64)>, String> {
    let con = load::open(db)?;
    let mut st = con.prepare(&format!("SELECT edge_id, length_m::DOUBLE FROM {mode}.edges")).map_err(|e| e.to_string())?;
    let mut rows = st.query([]).map_err(|e| e.to_string())?;
    let mut len_of = vec![0.0f64; net.graph.n_nodes()];
    while let Some(r) = rows.next().map_err(|e| e.to_string())? {
        let id: i64 = r.get(0).map_err(|e| e.to_string())?;
        if let Some(i) = net.graph.index_of(id) {
            len_of[i] = r.get::<_, Option<f64>>(1).map_err(|e| e.to_string())?.unwrap_or(0.0);
        }
    }
    Ok(net.turns.iter().map(|&(u, w, _)| (u, w, len_of[u as usize])).collect())
}

/// `build --cch [--metrics time,length]`: the structure with no costs and one folder of costs for each metric.
fn build_cch(args: &[String], net: &load::Network, db: &std::path::Path, mode: &str, top: u8, xyz: bool, dir: &std::path::Path) -> Result<(), String> {
    use tileroute::cch::Cch;
    let names: Vec<String> = arg(args, "--metrics").map_or("time", |s| s.as_str()).split(',').map(String::from).collect();
    let t = Instant::now();
    let c = Cch::new(net.graph.n_nodes(), net.turns.iter().map(|a| (a.0, a.1)), &net.level, &net.cell);
    println!("structure: {} edges, {:.2} s", c.n_edges(), t.elapsed().as_secs_f64());
    // mixNN: NN% time and (100 - NN)% length, each divided by its mean over all arcs (written to means.txt)
    let length = arc_lengths(net, db, mode)?;
    let k = net.turns.len().max(1) as f64;
    let (mt, ml) = (net.turns.iter().map(|a| a.2).sum::<f64>() / k, length.iter().map(|a| a.2).sum::<f64>() / k);
    let mut costs = Vec::new();
    for name in &names {
        costs.push(match name.as_str() {
            "time" => net.turns.clone(),
            "length" => length.clone(),
            m if m.starts_with("mix") => {
                let l: f64 = m[3..].parse::<f64>().map_err(|e| format!("{m}: {e}"))? / 100.0;
                net.turns.iter().zip(&length).map(|(a, b)| (a.0, a.1, l * a.2 / mt + (1.0 - l) * b.2 / ml)).collect()
            }
            m => return Err(format!("unknown metric {m} (time, length, mixNN)")),
        });
    }
    let metrics: Vec<_> = costs
        .iter()
        .zip(&names)
        .map(|(a, name)| {
            let t = Instant::now();
            let m = c.customize(a, true);
            println!("metric {name}: customization {:.2} s", t.elapsed().as_secs_f64());
            m
        })
        .collect();
    let t = Instant::now();
    let named: Vec<(&str, &tileroute::cch::Metric)> = names.iter().map(String::as_str).zip(metrics.iter()).collect();
    let items = node_items(net, db, mode)?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("means.txt"), format!("{mt} {ml}\n")).map_err(|e| e.to_string())?;
    let bytes = tileroute::cchdisk::write(&c, &named, &net.graph.ids, &items, &net.level, &net.cell, xyz.then_some(&net.home[..]), top, args.iter().any(|a| a == "--middle-nodes"), dir).map_err(|e| e.to_string())?;
    let size = |sub: &str| -> u64 { std::fs::read_dir(dir.join(sub)).map(|d| d.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum()).unwrap_or(0) };
    println!("wrote {}: {:.1} MB in all, {:.2} s; structure {:.1} MB", dir.display(), bytes as f64 / 1e6, t.elapsed().as_secs_f64(), size("structure") as f64 / 1e6);
    for name in &names {
        println!("  metric {name}: {:.1} MB", size(name) as f64 / 1e6);
    }
    Ok(())
}

/// `tileroute weigh <folder> --pairs-file CSV --candidates time,length,mix50 --exact mix30,mix70`: the weighted query
/// from a folder of `build --cch`. For a mix l, the cost of a route is l * time / mean time + (1 - l) * length / mean
/// length. The candidate metrics are searched in parallel and the route with the smallest cost for l wins; the exact
/// metric of the same l gives the error.
fn weigh(args: &[String]) -> Result<(), String> {
    use rayon::prelude::*;
    use tileroute::cchdisk::Store;
    let usage = "usage: tileroute weigh <folder> --pairs-file CSV --candidates time,length,mix50 --exact mix30,mix70";
    let (Some(dir), Some(path)) = (args.get(1), arg(args, "--pairs-file")) else { return Err(usage.into()) };
    let dir = std::path::Path::new(dir);
    let means = std::fs::read_to_string(dir.join("means.txt")).map_err(|e| format!("means.txt: {e}"))?;
    let m: Vec<f64> = means.split_whitespace().map(|x| x.parse().map_err(|e| format!("means.txt: {e}"))).collect::<Result<_, _>>()?;
    let (mt, ml) = (m[0], m[1]);
    let open = |list: &str| -> Result<Vec<(String, Store)>, String> {
        list.split(',').map(|n| Store::open(dir, n).map(|s| (n.to_string(), s)).map_err(|e| e.to_string())).collect()
    };
    let cands = open(arg(args, "--candidates").map_or("time,length,mix50", |s| s.as_str()))?;
    let exact = open(arg(args, "--exact").map_or("mix30,mix70", |s| s.as_str()))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let pairs: Vec<(i64, i64)> = text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split(',').map(|x| x.trim().parse::<i64>().ok());
            Some((f.next()??, f.next()??))
        })
        .collect();
    // the routes of the candidates, with their items; the wait time of one query: the candidates in parallel
    let t = Instant::now();
    let found: Vec<Vec<Option<[f64; 2]>>> = pairs
        .iter()
        .map(|&(a, b)| {
            cands.par_iter().map(|(_, s)| s.route_items(a, b).ok().and_then(|r| r.0).map(|r| r.2)).collect()
        })
        .collect();
    let wait = t.elapsed().as_secs_f64() / pairs.len().max(1) as f64 * 1e3;
    let t = Instant::now();
    for &(a, b) in &pairs {
        std::hint::black_box(cands[0].1.route(a, b).ok());
    }
    let one = t.elapsed().as_secs_f64() / pairs.len().max(1) as f64 * 1e3;
    println!(
        "weighted query: {} candidates ({}) searched in parallel: {wait:.3} ms for each query; one search alone: {one:.3} ms",
        cands.len(),
        cands.iter().map(|c| c.0.as_str()).collect::<Vec<_>>().join(", ")
    );
    for (name, s) in &exact {
        let l: f64 = name.strip_prefix("mix").and_then(|x| x.parse::<f64>().ok()).ok_or(format!("{name}: not a mixNN metric"))? / 100.0;
        let cost = |x: &[f64; 2]| l * x[0] / mt + (1.0 - l) * x[1] / ml;
        let (mut n, mut same, mut sum, mut max) = (0usize, 0usize, 0.0f64, 0.0f64);
        for (i, &(a, b)) in pairs.iter().enumerate() {
            let Some((best, _)) = s.route(a, b).map_err(|e| e.to_string())?.0 else { continue };
            if best <= 0.0 {
                continue;
            }
            let est = found[i].iter().flatten().map(cost).fold(f64::INFINITY, f64::min);
            let err = ((est - best) / best).max(0.0);
            n += 1;
            same += (err <= 1e-9) as usize;
            sum += err;
            max = max.max(err);
        }
        println!(
            "l = {l:.2} ({name}): {n} routes | estimate exact {:.1}% | mean error {:.3}% | largest error {:.2}%",
            same as f64 * 100.0 / n.max(1) as f64,
            sum * 100.0 / n.max(1) as f64,
            max * 100.0
        );
    }
    Ok(())
}

/// `tileroute reference <db> <pairs.csv> <out.csv> [--mode driving] [--max N]`: the cost of each pair with duckOSM's
/// router only (-1: no path), as a check for an area that is too large for `check`.
fn reference(args: &[String]) -> Result<(), String> {
    let (Some(db), Some(path), Some(out)) = (args.get(1), args.get(2), args.get(3)) else {
        return Err("usage: tileroute reference <db> <pairs.csv> <out.csv> [--mode driving] [--max N]".into());
    };
    let mode = arg(args, "--mode").map_or("driving", |s| s.as_str());
    let max: usize = arg(args, "--max").map_or(Ok(usize::MAX), |s| s.parse()).map_err(|e| format!("--max: {e}"))?;
    let t = Instant::now();
    let g = load::graph(&load::open(db.as_ref())?, mode)?;
    println!("duckOSM's graph: {} nodes, {:.1} s", g.n_nodes(), t.elapsed().as_secs_f64());
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    let pairs: Vec<(i64, i64)> = text
        .lines()
        .skip(1)
        .filter_map(|l| {
            let mut f = l.split(',').map(|x| x.trim().parse::<i64>().ok());
            Some((f.next()??, f.next()??))
        })
        .take(max)
        .collect();
    let t = Instant::now();
    let costs: Vec<f64> = pairs.par_iter().map(|&(a, b)| g.bidirectional_dijkstra(a, b).ok().flatten().map_or(-1.0, |r| r.0)).collect();
    let mut csv = String::from("source,target,cost\n");
    for (&(a, b), c) in pairs.iter().zip(&costs) {
        csv.push_str(&format!("{a},{b},{c}\n"));
    }
    std::fs::write(out, csv).map_err(|e| format!("{out}: {e}"))?;
    println!("{} pairs in {:.1} s (all threads); peak memory {:.0} MB", pairs.len(), t.elapsed().as_secs_f64(), disk::peak_rss() as f64 / 1e6);
    Ok(())
}
