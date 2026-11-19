//! `tileroute check <db.duckdb> [--mode driving] [--finest 20] [--pairs N] [--no-reference] [--write-pairs FILE]`:
//! make the shortcuts, then compare routes with duckOSM's router (all pairs, or N random pairs).

use rayon::prelude::*;
use std::time::Instant;
use tileroute::{cch::Cch, hier, load};

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |name: &str, default: &str| -> String {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned().unwrap_or(default.into())
    };
    let (Some("check"), Some(db)) = (args.first().map(String::as_str), args.get(1)) else {
        return Err("usage: tileroute check <db.duckdb> [--mode driving] [--finest 20] [--pairs N]".into());
    };
    let mode = opt("--mode", "driving");
    let finest: u8 = opt("--finest", "20").parse().map_err(|e| format!("--finest: {e}"))?;

    let t = Instant::now();
    let net = load::read(db.as_ref(), &mode, finest)?;
    let n = net.graph.n_nodes();
    println!("read: edge graph with {n} nodes (road edges) and {} arcs (edge_graph rows), {:.2} s", net.turns.len(), t.elapsed().as_secs_f64());

    let t = Instant::now();
    let mut h = hier::prepare(&net.turns, net.level.clone(), net.cell.clone());
    println!("shortcuts: {} made, {} arcs in all, {:.3} s", h.shortcuts.iter().sum::<usize>(), h.n_arcs(), t.elapsed().as_secs_f64());
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

    let lca = args.iter().any(|a| a == "--lca");
    if lca {
        let t = Instant::now();
        h.top_down();
        let (tiles, exits, cells) = h.table_size();
        println!("top-down: {tiles} tile tables, {exits} exits, {cells} table cells, {:.3} s", t.elapsed().as_secs_f64());
    }
    let h = h;
    // the triangle version (a customizable contraction hierarchy, order from the tiles)
    let tri = if args.iter().any(|a| a == "--cch") {
        let t = Instant::now();
        let mut c = Cch::new(n, &net.turns, &net.level, &net.cell);
        let all = t.elapsed().as_secs_f64();
        let t = Instant::now();
        c.customize();
        let cust = t.elapsed().as_secs_f64();
        let (edges, only) = c.size();
        println!(
            "triangles: {edges} edges ({only} shortcuts only), structure and first costs {all:.3} s, a full customization {cust:.3} s"
        );
        Some(c)
    } else {
        None
    };
    let route = |s: u32, t: u32| match &tri {
        Some(c) => c.route(s, t),
        None if lca => h.route_lca(s, t).0,
        None => h.route(s, t),
    };

    // the pairs: all, or N random ones (a fixed sequence)
    let pairs: Vec<(u32, u32)> = match opt("--pairs", "").parse::<usize>() {
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
    };
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
    let tiles_up = mean(&|s, t| h.route_stats(s, t).1);
    if lca {
        println!("tiles per query: {tiles_up:.1} (search up), {:.1} (top-down, pruned at the LCA)", mean(&|s, t| h.route_lca(s, t).1));
    } else {
        println!("tiles per query: {tiles_up:.1}");
    }
    let ids = &net.graph.ids;
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
    if !bad.is_empty() {
        return Err(format!("{} wrong routes", bad.len()));
    }
    if let (Some(c), Some(list)) = (&tri, args.iter().position(|a| a == "--changes").and_then(|i| args.get(i + 1))) {
        for k in list.split(',') {
            let k: usize = k.parse().map_err(|e| format!("--changes: {e}"))?;
            update_test(c, &net, k)?;
        }
    }
    Ok(())
}

/// Change the travel time of `k` random road edges (the cost of all arcs out of each: half of them x 1.5, the other
/// half x 0.6), update the costs, and check: the same costs as a full customization, and the same routes as duckOSM's
/// router on the changed graph.
fn update_test(c: &Cch, net: &load::Network, k: usize) -> Result<(), String> {
    let n = net.graph.n_nodes();
    let mut x: u64 = 0xD1B5_4A32_D192_ED03 ^ k as u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut factor: std::collections::HashMap<u32, f64> = std::collections::HashMap::new();
    while factor.len() < k.min(n) {
        let v = (next() % n as u64) as u32;
        let f = if factor.len() % 2 == 0 { 1.5 } else { 0.6 };
        factor.entry(v).or_insert(f);
    }
    // the new arc costs: f32 values, as in the database
    let turns: Vec<(u32, u32, f64)> = net
        .turns
        .iter()
        .map(|&(u, w, cost)| (u, w, factor.get(&u).map_or(cost, |f| (cost * f) as f32 as f64)))
        .collect();
    let changes: Vec<(u32, u32, f64)> = turns.iter().zip(&net.turns).filter(|(a, b)| a.2 != b.2).map(|(a, _)| *a).collect();

    let mut upd = c.clone();
    let t = Instant::now();
    let redone = upd.update(&changes);
    let t_upd = t.elapsed().as_secs_f64();
    let mut full = c.clone();
    for &(a, b, w) in &changes {
        full.set_base(a, b, w);
    }
    let t = Instant::now();
    full.customize();
    let t_full = t.elapsed().as_secs_f64();
    let same = upd.same_costs(&full);

    // routes on the changed graph against duckOSM's router with the same arcs
    let ids = &net.graph.ids;
    let g = duckosm_router::graph::Graph::from_arcs(turns.iter().map(|&(u, w, cost)| (ids[u as usize], ids[w as usize], cost)));
    let mut wrong = 0usize;
    for _ in 0..1000 {
        let (s, t) = ((next() % n as u64) as u32, (next() % n as u64) as u32);
        let ours = upd.route(s, t).map(|r| r.0);
        let theirs = g.bidirectional_dijkstra(ids[s as usize], ids[t as usize]).ok().flatten().map(|r| r.0);
        if ours != theirs {
            wrong += 1;
        }
    }
    println!(
        "update: {k} road edges ({} arcs) changed | {redone} edges computed again in {:.3} ms | full customization {:.1} ms | same costs: {same} | 1000 routes on the changed graph: {wrong} wrong",
        changes.len(),
        t_upd * 1e3,
        t_full * 1e3
    );
    if !same || wrong > 0 {
        return Err("the update is wrong".into());
    }
    Ok(())
}
