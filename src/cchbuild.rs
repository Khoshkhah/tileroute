//! The structure with no costs ([`crate::cch`]) and the costs of many metrics, built one cell at a time, as
//! `crate::bytile` builds the hierarchy: the memory is one cell and the small top, not the whole area. The files are
//! those of [`crate::cchdisk::write`].
//!
//! 1. The nodes: level, cell, unit (the cell of zoom `tz + 1`), file, rank and node number of each road edge.
//! 2. The arcs, as a stream, into one bucket file for each unit, and one for the top.
//! 3. The means of time and length over all arcs (for the metrics mixNN).
//! 4. For each unit: the structure of its nodes (the elimination), and the customization of all metrics. A fill edge
//!    or a triangle cost between two top nodes goes to the top.
//! 5. The top: its structure and its customization, with the fill edges and the triangle costs of all units.
//! 6. For each unit: the arc reduction together with the top (the mask), and the lower neighbours. Each node goes to the
//!    bucket of its file. Then each file is written from its bucket.

use crate::bytile::read_bucket;
use crate::cch::Cch;
use crate::cchdisk::{write_file, FileData};
use crate::hier::{reduce, reduce_from, Fast};
use crate::{disk, load, tile};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// What the build did.
pub struct Report {
    pub nodes: usize,
    pub arcs: usize,
    pub files: usize,
    pub top_nodes: usize,
    pub edges: usize,
    pub bytes: u64,
    pub times: Vec<(&'static str, f64)>,
}

/// A metric: time, length, or l · time / mean time + (1 − l) · length / mean length.
#[derive(Clone, Copy)]
enum Kind {
    Time,
    Length,
    Mix(f64),
}

fn kind(name: &str) -> Result<Kind, String> {
    match name {
        "time" => Ok(Kind::Time),
        "length" => Ok(Kind::Length),
        m if m.starts_with("mix") => Ok(Kind::Mix(m[3..].parse::<f64>().map_err(|e| format!("{m}: {e}"))? / 100.0)),
        m => Err(format!("unknown metric {m} (time, length, mixNN)")),
    }
}

/// One edge of a unit or of the top: the lower node, the higher node (node numbers), and for each metric the cost up
/// (lower → higher) and down.
struct Row {
    lo: u32,
    hi: u32,
    cost: Vec<[f64; 2]>,
}

fn write_rows(f: &mut impl Write, rows: &[Row]) -> io::Result<()> {
    for r in rows {
        f.write_all(&r.lo.to_le_bytes())?;
        f.write_all(&r.hi.to_le_bytes())?;
        for c in &r.cost {
            f.write_all(&c[0].to_le_bytes())?;
            f.write_all(&c[1].to_le_bytes())?;
        }
    }
    Ok(())
}

fn read_rows(path: &Path, m: usize) -> io::Result<Vec<Row>> {
    let mut b = Vec::new();
    BufReader::new(File::open(path)?).read_to_end(&mut b)?;
    let size = 8 + 16 * m;
    let f64_at = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
    Ok(b.chunks_exact(size)
        .enumerate()
        .map(|(n, c)| {
            let i = n * size;
            Row {
                lo: u32::from_le_bytes(c[0..4].try_into().expect("4")),
                hi: u32::from_le_bytes(c[4..8].try_into().expect("4")),
                cost: (0..m).map(|k| [f64_at(i + 8 + 16 * k), f64_at(i + 16 + 16 * k)]).collect(),
            }
        })
        .collect())
}

/// One node for the file buckets: number, rank, edge id, items, higher neighbours with (for each metric) the costs and
/// the two mask bits, lower neighbours.
struct NodeRec {
    num: u32,
    rank: u32,
    edge: i64,
    item: [f32; 2],
    up: Vec<(u32, Vec<(f64, f64, bool, bool)>)>,
    lower: Vec<u32>,
}

fn write_node(f: &mut impl Write, r: &NodeRec) -> io::Result<()> {
    f.write_all(&r.num.to_le_bytes())?;
    f.write_all(&r.rank.to_le_bytes())?;
    f.write_all(&r.edge.to_le_bytes())?;
    f.write_all(&r.item[0].to_le_bytes())?;
    f.write_all(&r.item[1].to_le_bytes())?;
    f.write_all(&(r.up.len() as u32).to_le_bytes())?;
    for (w, ms) in &r.up {
        f.write_all(&w.to_le_bytes())?;
        for &(u, d, ku, kd) in ms {
            f.write_all(&u.to_le_bytes())?;
            f.write_all(&d.to_le_bytes())?;
            f.write_all(&[ku as u8, kd as u8])?;
        }
    }
    f.write_all(&(r.lower.len() as u32).to_le_bytes())?;
    for x in &r.lower {
        f.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

fn read_nodes(path: &Path, m: usize) -> io::Result<Vec<NodeRec>> {
    let mut b = Vec::new();
    if path.exists() {
        BufReader::new(File::open(path)?).read_to_end(&mut b)?;
    }
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    let f64_at = |i: usize| f64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
    let f32_at = |i: usize| f32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (num, rank) = (u32_at(i), u32_at(i + 4));
        let edge = i64::from_le_bytes(b[i + 8..i + 16].try_into().expect("8 bytes"));
        let item = [f32_at(i + 16), f32_at(i + 20)];
        let k = u32_at(i + 24) as usize;
        i += 28;
        let mut up = Vec::with_capacity(k);
        for _ in 0..k {
            let w = u32_at(i);
            i += 4;
            let ms = (0..m).map(|j| (f64_at(i + 18 * j), f64_at(i + 18 * j + 8), b[i + 18 * j + 16] == 1, b[i + 18 * j + 17] == 1)).collect();
            i += 18 * m;
            up.push((w, ms));
        }
        let k = u32_at(i) as usize;
        i += 4;
        let lower = (0..k).map(|j| u32_at(i + 4 * j)).collect();
        i += 4 * k;
        out.push(NodeRec { num, rank, edge, item, up, lower });
    }
    Ok(out)
}

/// Build the folder of `build --cch` one unit (cell of zoom `tz + 1`) at a time, with the cells from the partition.
#[allow(clippy::too_many_arguments)]
pub fn build(db: &Path, mode: &str, finest: u8, tz: u8, leaf: usize, xyz: bool, names: &[String], dir: &Path) -> Result<Report, String> {
    let kinds: Vec<Kind> = names.iter().map(|n| kind(n)).collect::<Result<_, _>>()?;
    let nm = kinds.len();
    let mut times = Vec::new();
    let t = std::time::Instant::now();
    std::fs::create_dir_all(dir).map_err(err)?;
    let tmp: PathBuf = dir.join(".build");
    std::fs::create_dir_all(&tmp).map_err(err)?;
    let con = load::open(db)?;
    let relation = load::relation(&con, mode)?;

    // 1. the nodes: (edge id, level, cell, zoom-8 XYZ tile), then the edges of the graph that are not in `edges`
    let mut nodes: Vec<(i64, u8, u64, u64)> = Vec::new();
    load::partition_levels(&con, mode, finest, leaf, |id, z, c, h| nodes.push((id, z, c, h)))?;
    {
        let mut st = con
            .prepare(&format!(
                "SELECT DISTINCT x FROM (SELECT from_edge AS x FROM {relation} UNION SELECT to_edge FROM {relation})
                 WHERE x NOT IN (SELECT edge_id FROM {mode}.edges)"
            ))
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            nodes.push((r.get(0).map_err(err)?, 0, 0, 0));
        }
    }
    nodes.par_sort_unstable_by_key(|x| x.0);
    nodes.dedup_by_key(|x| x.0);
    let n = nodes.len();
    // the length of each road edge (by the index in `nodes`)
    let mut len_i = vec![0f32; n];
    {
        let mut st = con.prepare(&format!("SELECT edge_id, length_m::DOUBLE FROM {mode}.edges")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let id: i64 = r.get(0).map_err(err)?;
            if let Ok(i) = nodes.binary_search_by_key(&id, |x| x.0) {
                len_i[i] = r.get::<_, Option<f64>>(1).map_err(err)?.unwrap_or(0.0) as f32;
            }
        }
    }
    let unit_of = |z: u8, c: u64| if z <= tz { 0 } else { tile::parent(c, tz + 1) };
    let file_of = |z: u8, c: u64, h: u64| if z <= tz { 0 } else if xyz { h } else { unit_of(z, c) };
    // the rank: the finest level first, then by cell, then by index (as `Cch::new`)
    let mut by_rank: Vec<u32> = (0..n as u32).collect();
    by_rank.par_sort_unstable_by_key(|&i| { let (_, z, c, _) = nodes[i as usize]; (std::cmp::Reverse(z), c, i) });
    let mut rank_i = vec![0u32; n];
    for (r, &i) in by_rank.iter().enumerate() {
        rank_i[i as usize] = r as u32;
    }
    drop(by_rank);
    // the node numbers: by file (the top first), then by rank
    let mut order: Vec<u32> = (0..n as u32).collect();
    order.par_sort_unstable_by_key(|&i| { let (_, z, c, h) = nodes[i as usize]; (file_of(z, c, h), rank_i[i as usize]) });
    let mut num_i = vec![0u32; n];
    for (k, &i) in order.iter().enumerate() {
        num_i[i as usize] = k as u32;
    }
    // by node number
    let level: Vec<u8> = order.iter().map(|&i| nodes[i as usize].1).collect();
    let cell: Vec<u64> = order.iter().map(|&i| nodes[i as usize].2).collect();
    let edge: Vec<i64> = order.iter().map(|&i| nodes[i as usize].0).collect();
    let rank: Vec<u32> = order.iter().map(|&i| rank_i[i as usize]).collect();
    let len: Vec<f32> = order.iter().map(|&i| len_i[i as usize]).collect();
    let file_key: Vec<u64> = order.iter().map(|&i| { let (_, z, c, h) = nodes[i as usize]; file_of(z, c, h) }).collect();
    let unit: Vec<u64> = order.iter().map(|&i| { let (_, z, c, _) = nodes[i as usize]; unit_of(z, c) }).collect();
    let ids: Vec<i64> = nodes.iter().map(|x| x.0).collect();
    drop((nodes, len_i, rank_i, order));
    let node_of = |id: i64| ids.binary_search(&id).ok().map(|i| num_i[i]);
    let mut files: Vec<(u32, u32, u64)> = Vec::new();
    for v in 0..n as u32 {
        let key = file_key[v as usize];
        match files.last_mut() {
            Some(f) if f.2 == key => f.1 += 1,
            _ => files.push((v, 1, key)),
        }
    }
    if files.first().is_none_or(|f| f.2 != 0) {
        files.insert(0, (0, 0, 0));
    }
    drop(file_key);
    let ntop = files[0].1 as usize;
    // the units: the nodes below the top, by unit and in a unit by rank
    let mut unit_nodes: Vec<u32> = (ntop as u32..n as u32).collect();
    unit_nodes.par_sort_unstable_by_key(|&v| (unit[v as usize], rank[v as usize]));
    let mut units: Vec<(u64, std::ops::Range<usize>)> = Vec::new();
    for (i, &v) in unit_nodes.iter().enumerate() {
        let key = unit[v as usize];
        match units.last_mut() {
            Some(u) if u.0 == key => u.1.end = i + 1,
            _ => units.push((key, i..i + 1)),
        }
    }
    times.push(("nodes (with the partition)", t.elapsed().as_secs_f64()));

    // 2. the arcs into the bucket of their unit (or the top); the time of each node is the cost of its arcs
    let t = std::time::Instant::now();
    let bucket = |key: u64| tmp.join(format!("arcs_{key}.bin"));
    let mut time = vec![0f32; n];
    let mut arcs = 0usize;
    {
        let mut writers: HashMap<u64, BufWriter<File>> = HashMap::new();
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {relation}")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        let mut seq = 0u64;
        while let Some(r) = rows.next().map_err(err)? {
            let (a, b): (i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
            let c: Option<f64> = r.get(2).map_err(err)?;
            let (Some(u), Some(w)) = (node_of(a), node_of(b)) else { continue };
            if let Some(c) = c {
                time[u as usize] = c as f32;
            }
            let key = if unit[u as usize] != 0 { unit[u as usize] } else { unit[w as usize] };
            let f = match writers.entry(key) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(BufWriter::new(File::create(bucket(key)).map_err(err)?)),
            };
            let mut rec = [0u8; 24];
            rec[0..4].copy_from_slice(&u.to_le_bytes());
            rec[4..8].copy_from_slice(&w.to_le_bytes());
            rec[8..16].copy_from_slice(&c.unwrap_or(f64::NAN).to_le_bytes());
            rec[16..24].copy_from_slice(&seq.to_le_bytes());
            f.write_all(&rec).map_err(err)?;
            seq += 1;
            arcs += 1;
        }
        for (_, mut f) in writers.drain() {
            f.flush().map_err(err)?;
        }
    }
    drop(con);
    times.push(("arcs into buckets", t.elapsed().as_secs_f64()));

    // 3. the means of time and length over all arcs (each pair once, as the graph)
    let t = std::time::Instant::now();
    let (mut st_, mut sl, mut k) = (0f64, 0f64, 0usize);
    for key in std::iter::once(0).chain(units.iter().map(|u| u.0)) {
        for (u, _, c) in read_bucket(&bucket(key)).map_err(err)? {
            st_ += c;
            sl += len[u as usize] as f64;
            k += 1;
        }
    }
    let (mt, ml) = (st_ / k.max(1) as f64, sl / k.max(1) as f64);
    std::fs::write(dir.join("means.txt"), format!("{mt} {ml}\n")).map_err(err)?;
    let cost = |m: usize, u: u32, c: f64| -> f64 {
        match kinds[m] {
            Kind::Time => c,
            Kind::Length => len[u as usize] as f64,
            Kind::Mix(l) => l * c / mt + (1.0 - l) * len[u as usize] as f64 / ml,
        }
    };
    times.push(("means", t.elapsed().as_secs_f64()));

    // 4. each unit: its structure and its customization
    let t = std::time::Instant::now();
    let unit_file = |key: u64| tmp.join(format!("unit_{key}.bin"));
    // between two top nodes: the fill edges, and the best triangle costs (for each metric: up, down)
    let mut top_fill: Vec<(u32, u32)> = Vec::new();
    let mut top_tri: HashMap<(u32, u32), Vec<[f64; 2]>> = HashMap::new();
    let mut edges = 0usize;
    for (key, range) in &units {
        let vs = &unit_nodes[range.clone()];
        let k = vs.len();
        // local numbers: the nodes of the unit 0..k (by rank), then the top nodes that its arcs touch
        let mut glob: Vec<u32> = vs.to_vec();
        let mut loc: HashMap<u32, u32> = vs.iter().enumerate().map(|(i, &v)| (v, i as u32)).collect();
        let arcs_u = read_bucket(&bucket(*key)).map_err(err)?;
        std::fs::remove_file(bucket(*key)).map_err(err)?;
        let mut local = |v: u32, glob: &mut Vec<u32>| -> u32 {
            *loc.entry(v).or_insert_with(|| {
                glob.push(v);
                glob.len() as u32 - 1
            })
        };
        let arcs_l: Vec<(u32, u32, u32, f64)> = arcs_u.iter().map(|&(u, w, c)| (local(u, &mut glob), local(w, &mut glob), u, c)).collect();
        // the order of local nodes: the unit by local number, the top after it by rank
        let ord = |l: u32| -> u64 { if (l as usize) < k { l as u64 } else { (1 << 40) + rank[glob[l as usize] as usize] as u64 } };
        let mut up: Vec<Vec<u32>> = vec![Vec::new(); k];
        for &(a, b, _, _) in &arcs_l {
            let (lo, hi) = if ord(a) < ord(b) { (a, b) } else { (b, a) };
            if (lo as usize) < k {
                up[lo as usize].push(hi);
            }
        }
        // the elimination: the lowest higher neighbour gets the other higher neighbours
        for x in 0..k {
            let mut row = std::mem::take(&mut up[x]);
            row.sort_unstable_by_key(|&l| ord(l));
            row.dedup();
            if let Some(&p) = row.first() {
                if (p as usize) < k {
                    up[p as usize].extend_from_slice(&row[1..]);
                } else {
                    top_fill.extend(row[1..].iter().map(|&r| (glob[p as usize], glob[r as usize])));
                }
            }
            up[x] = row;
        }
        let mut off = vec![0usize];
        let mut other: Vec<u32> = Vec::new();
        for row in &up {
            other.extend_from_slice(row);
            off.push(other.len());
        }
        drop(up);
        edges += other.len();
        let okey: Vec<u64> = other.iter().map(|&l| ord(l)).collect();
        let edge_of = |a: usize, b: u32| -> usize { off[a] + okey[off[a]..off[a + 1]].binary_search(&ord(b)).expect("an edge of the unit") };
        // the customization of all metrics: cu = lower → higher, cd = higher → lower
        let mut cu = vec![vec![f64::INFINITY; other.len()]; nm];
        let mut cd = vec![vec![f64::INFINITY; other.len()]; nm];
        for &(a, b, u, c) in &arcs_l {
            let (lo, hi, upward) = if ord(a) < ord(b) { (a, b, true) } else { (b, a, false) };
            let e = edge_of(lo as usize, hi);
            for m in 0..nm {
                let x = if upward { &mut cu[m][e] } else { &mut cd[m][e] };
                *x = x.min(cost(m, u, c));
            }
        }
        for x in 0..k {
            for i in off[x]..off[x + 1] {
                let y = other[i];
                for j in i + 1..off[x + 1] {
                    let z = other[j];
                    if (y as usize) < k {
                        let e = edge_of(y as usize, z);
                        for m in 0..nm {
                            cu[m][e] = cu[m][e].min(cd[m][i] + cu[m][j]);
                            cd[m][e] = cd[m][e].min(cd[m][j] + cu[m][i]);
                        }
                    } else {
                        let t = top_tri.entry((glob[y as usize], glob[z as usize])).or_insert_with(|| vec![[f64::INFINITY; 2]; nm]);
                        for m in 0..nm {
                            t[m][0] = t[m][0].min(cd[m][i] + cu[m][j]);
                            t[m][1] = t[m][1].min(cd[m][j] + cu[m][i]);
                        }
                    }
                }
            }
        }
        let rows: Vec<Row> = (0..k)
            .flat_map(|x| (off[x]..off[x + 1]).map(move |i| (x, i)))
            .map(|(x, i)| Row { lo: glob[x], hi: glob[other[i] as usize], cost: (0..nm).map(|m| [cu[m][i], cd[m][i]]).collect() })
            .collect();
        let mut f = BufWriter::new(File::create(unit_file(*key)).map_err(err)?);
        write_rows(&mut f, &rows).map_err(err)?;
        f.flush().map_err(err)?;
    }
    times.push(("units: structure and customization", t.elapsed().as_secs_f64()));

    // 5. the top: structure and customization with the fill edges and the triangle costs of the units
    let t = std::time::Instant::now();
    let top_arcs = read_bucket(&bucket(0)).map_err(err)?;
    let _ = std::fs::remove_file(bucket(0));
    let pairs = top_arcs.iter().map(|a| (a.0, a.1)).chain(top_fill.iter().copied()).chain(top_tri.keys().copied());
    let top = Cch::new(ntop, pairs, &level[..ntop], &cell[..ntop]);
    edges += top.n_edges();
    let top_m: Vec<_> = (0..nm)
        .map(|m| {
            let mut a: Vec<(u32, u32, f64)> = top_arcs.iter().map(|&(u, w, c)| (u, w, cost(m, u, c))).collect();
            for (&(y, z), c) in &top_tri {
                a.push((y, z, c[m][0]));
                a.push((z, y, c[m][1]));
            }
            top.customize(&a, false)
        })
        .collect();
    drop((top_fill, top_tri));
    // the search graphs of the top for each metric (rows by node number; the top numbers are 0..ntop)
    let top_rows = |m: usize, dir: usize| -> Vec<Vec<(f64, u32)>> {
        let mut rows = vec![Vec::new(); ntop];
        for r in 0..ntop as u32 {
            let v = top.order[r as usize];
            for e in top.row(r) {
                let c = if dir == 0 { top_m[m].up[e] } else { top_m[m].down[e] };
                if c.is_finite() {
                    rows[v as usize].push((c, top.order[top.up[e] as usize]));
                }
            }
        }
        rows
    };
    let flat = |rows: &[Vec<(f64, u32)>]| -> Fast {
        let mut off = vec![0u32];
        let mut arc = Vec::new();
        for r in rows {
            arc.extend_from_slice(r);
            off.push(arc.len() as u32);
        }
        Fast { off, arc }
    };
    let top_search: Vec<[Vec<Vec<(f64, u32)>>; 2]> = (0..nm).map(|m| [top_rows(m, 0), top_rows(m, 1)]).collect();
    times.push(("top: structure and customization", t.elapsed().as_secs_f64()));

    // 6. each unit: the mask (arc reduction together with the top), the lower neighbours, the nodes into the file buckets
    let t = std::time::Instant::now();
    let part = |key: u64| tmp.join(format!("file_{key}.bin"));
    let mut parts: HashMap<u64, BufWriter<File>> = HashMap::new();
    let file_of_num = |v: u32| files[files.partition_point(|f| f.0 <= v) - 1].2;
    let mut top_lower: Vec<Vec<u32>> = vec![Vec::new(); ntop];
    let item = |v: u32| [time[v as usize], len[v as usize]];
    for (key, range) in &units {
        let vs = &unit_nodes[range.clone()];
        let k = vs.len();
        let rows = read_rows(&unit_file(*key), nm).map_err(err)?;
        std::fs::remove_file(unit_file(*key)).map_err(err)?;
        // local space: the top 0..ntop, then the unit by rank
        let li: HashMap<u32, usize> = vs.iter().enumerate().map(|(i, &v)| (v, ntop + i)).collect();
        let loc = |g: u32| -> usize { if (g as usize) < ntop { g as usize } else { li[&g] } };
        // keep[m * 2 + dir][row index]: each metric and direction in parallel
        let keep: Vec<Vec<bool>> = (0..2 * nm)
            .into_par_iter()
            .map(|md| {
                let (m, dir) = (md / 2, md % 2);
                let mut g = top_search[m][dir].clone();
                g.resize(ntop + k, Vec::new());
                for r in &rows {
                    let c = r.cost[m][dir];
                    if c.is_finite() {
                        g[loc(r.lo)].push((c, loc(r.hi) as u32));
                    }
                }
                let red = reduce_from(flat(&g), ntop);
                rows.iter().map(|r| { let w = loc(r.hi) as u32; red.of(loc(r.lo) as u32).iter().any(|&(_, x)| x == w) }).collect()
            })
            .collect();
        let mut recs: HashMap<u32, NodeRec> = vs.iter().map(|&v| (v, NodeRec { num: v, rank: rank[v as usize], edge: edge[v as usize], item: item(v), up: Vec::new(), lower: Vec::new() })).collect();
        for (j, r) in rows.iter().enumerate() {
            recs.get_mut(&r.lo).expect("a node of the unit").up.push((r.hi, (0..nm).map(|m| (r.cost[m][0], r.cost[m][1], keep[2 * m][j], keep[2 * m + 1][j])).collect()));
            if (r.hi as usize) < ntop {
                top_lower[r.hi as usize].push(r.lo);
            } else {
                recs.get_mut(&r.hi).expect("a node of the unit").lower.push(r.lo);
            }
        }
        for v in vs {
            let rec = &recs[v];
            let fk = file_of_num(*v);
            let f = match parts.entry(fk) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => e.insert(BufWriter::new(File::create(part(fk)).map_err(err)?)),
            };
            write_node(f, rec).map_err(err)?;
        }
    }
    // the top nodes: their mask from the top alone (the rows of the units are not witnesses of top arcs)
    {
        let red: Vec<[Fast; 2]> = (0..nm).map(|m| [reduce(flat(&top_search[m][0])), reduce(flat(&top_search[m][1]))]).collect();
        let kept = |m: usize, dir: usize, v: u32, w: u32| red[m][dir].of(v).iter().any(|&(_, x)| x == w);
        let f = parts.entry(0).or_insert_with(|| BufWriter::new(File::create(part(0)).expect("a file bucket")));
        for v in 0..ntop as u32 {
            let r = top.rank[v as usize];
            let mut rec = NodeRec { num: v, rank: rank[v as usize], edge: edge[v as usize], item: item(v), up: Vec::new(), lower: std::mem::take(&mut top_lower[v as usize]) };
            for e in top.row(r) {
                let w = top.order[top.up[e] as usize];
                rec.up.push((w, (0..nm).map(|m| (top_m[m].up[e], top_m[m].down[e], kept(m, 0, v, w), kept(m, 1, v, w))).collect()));
                let _ = e;
            }
            // the lower neighbours inside the top
            write_node(f, &rec).map_err(err)?;
        }
    }
    for (_, mut f) in parts.drain() {
        f.flush().map_err(err)?;
    }
    // the lower neighbours of top nodes inside the top: from the rows of the top
    let mut top_low_top: Vec<Vec<u32>> = vec![Vec::new(); ntop];
    for r in 0..ntop as u32 {
        let v = top.order[r as usize];
        for e in top.row(r) {
            top_low_top[top.order[top.up[e] as usize] as usize].push(v);
        }
    }
    let mut bytes = 0u64;
    let mnames: Vec<&str> = names.iter().map(String::as_str).collect();
    for &(first, k, key) in &files {
        let mut recs = read_nodes(&part(key), nm).map_err(err)?;
        let _ = std::fs::remove_file(part(key));
        recs.sort_unstable_by_key(|r| r.num);
        if recs.len() != k as usize {
            return Err(format!("file {key}: {} nodes, expected {k}", recs.len()));
        }
        let mut d = FileData { rank: Vec::new(), edge: Vec::new(), item: Vec::new(), off: vec![0], other: Vec::new(), doff: vec![0], down: Vec::new(), metrics: vec![(Vec::new(), Vec::new(), Vec::new(), Vec::new()); nm] };
        let mut bits: Vec<[Vec<bool>; 2]> = vec![[Vec::new(), Vec::new()]; nm];
        for mut r in recs {
            d.rank.push(r.rank);
            d.edge.push(r.edge);
            d.item.extend_from_slice(&r.item);
            r.up.sort_unstable_by_key(|x| x.0);
            for (w, ms) in &r.up {
                d.other.push(*w);
                for (m, &(u, dn, ku, kd)) in ms.iter().enumerate() {
                    d.metrics[m].0.push(u);
                    d.metrics[m].1.push(dn);
                    bits[m][0].push(ku);
                    bits[m][1].push(kd);
                }
            }
            d.off.push(d.other.len() as u32);
            if (r.num as usize) < ntop {
                r.lower.extend_from_slice(&top_low_top[r.num as usize]);
            }
            r.lower.sort_unstable();
            d.down.extend_from_slice(&r.lower);
            d.doff.push(d.down.len() as u32);
        }
        for (m, b) in bits.iter().enumerate() {
            let pack = |v: &[bool]| -> Vec<u64> {
                let mut out = vec![0u64; v.len().div_ceil(64)];
                for (j, &x) in v.iter().enumerate() {
                    if x {
                        out[j / 64] |= 1 << (j % 64);
                    }
                }
                out
            };
            d.metrics[m].2 = pack(&b[0]);
            d.metrics[m].3 = pack(&b[1]);
        }
        bytes += write_file(dir, &mnames, key, first, &d).map_err(err)?;
    }
    let mut pairs: Vec<(i64, u32)> = (0..n).map(|v| (edge[v], v as u32)).collect();
    pairs.par_sort_unstable();
    let b = disk::edges_bytes(&pairs);
    std::fs::write(dir.join("edges.idx"), &b).map_err(err)?;
    bytes += b.len() as u64;
    std::fs::write(dir.join("index.bin"), disk::index_bytes(&files)).map_err(err)?;
    std::fs::remove_dir_all(&tmp).map_err(err)?;
    times.push(("mask and files, unit by unit", t.elapsed().as_secs_f64()));
    Ok(Report { nodes: n, arcs, files: files.len() - 1, top_nodes: ntop, edges, bytes, times })
}
