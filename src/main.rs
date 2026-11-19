//! `tileroute check <db.duckdb> [--mode driving] [--finest 20] [--pairs N] [--no-reference] [--write-pairs FILE]`:
//! make the shortcuts, then compare routes with duckOSM's router (all pairs, or N random pairs).

use rayon::prelude::*;
use std::time::Instant;
use tileroute::{hier, load};

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
    let h = hier::prepare(&net.turns, net.level.clone(), net.cell.clone());
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

    let route = |s: u32, t: u32| h.route(s, t);

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
    println!("tiles per query: {:.1}", mean(&|s, t| h.route_stats(s, t).1));
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
    if bad.is_empty() { Ok(()) } else { Err(format!("{} wrong routes", bad.len())) }
}
