//! The preparation tile by tile (`tileroute build --by-tile`): it holds one zoom-8 tile at a time, not the whole
//! network. The files are the same as those of `Hierarchy::write_files` (see `docs/explanation/build-by-tile.md`).
//!
//! 1. The nodes: level, cell, file and final node number of each road edge.
//! 2. The arcs, as a stream, into bucket files: the bucket of the zoom-8 tile of a fine node, or the top bucket.
//! 3. For each zoom-8 tile: the steps of zoom 20 to 8 on its bucket; the result into a file of the tile.
//! 4. The top (level 7 and coarser): its arcs and the top shortcuts of all tiles; the steps of zoom 7 to 1.
//! 5. For each zoom-8 tile: arc reduction with the top, then its tile file; then the top file and the indexes.

use crate::disk::{self, AArc, SArc, TOP};
use crate::hier::{reduce, reduce_from, run_steps, Arc, Fast, Steps};
use crate::{load, tile};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// One arc with its saved inner nodes (node numbers).
struct Row {
    u: u32,
    w: u32,
    cost: f64,
    path: Vec<u32>,
}

fn write_row(f: &mut impl Write, r: &Row) -> io::Result<()> {
    f.write_all(&r.u.to_le_bytes())?;
    f.write_all(&r.w.to_le_bytes())?;
    f.write_all(&r.cost.to_le_bytes())?;
    f.write_all(&(r.path.len() as u32).to_le_bytes())?;
    for x in &r.path {
        f.write_all(&x.to_le_bytes())?;
    }
    Ok(())
}

fn read_rows(path: &Path) -> io::Result<Vec<Row>> {
    let mut b = Vec::new();
    if path.exists() {
        BufReader::new(File::open(path)?).read_to_end(&mut b)?;
    }
    let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("4 bytes"));
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let (u, w) = (u32_at(i), u32_at(i + 4));
        let cost = f64::from_le_bytes(b[i + 8..i + 16].try_into().expect("8 bytes"));
        let k = u32_at(i + 16) as usize;
        let path = (0..k).map(|j| u32_at(i + 20 + 4 * j)).collect();
        out.push(Row { u, w, cost, path });
        i += 20 + 4 * k;
    }
    Ok(out)
}

/// What the build did.
pub struct Report {
    pub nodes: usize,
    pub arcs: usize,
    pub tiles: usize,
    pub top_nodes: usize,
    pub bytes: u64,
    /// (phase, seconds)
    pub times: Vec<(&'static str, f64)>,
}

/// The local graph of one step run: node numbers are local (0..k); `global` gives the node number of each.
struct Local {
    global: Vec<u32>,
    index: HashMap<u32, u32>,
    level: Vec<u8>,
    cell: Vec<u64>,
    arcs: Vec<HashMap<u32, Arc>>,
    inn: Vec<Vec<u32>>,
}

impl Local {
    fn new() -> Local {
        Local { global: Vec::new(), index: HashMap::new(), level: Vec::new(), cell: Vec::new(), arcs: Vec::new(), inn: Vec::new() }
    }

    fn node(&mut self, g: u32, level: &[u8], cell: &[u64]) -> u32 {
        if let Some(&i) = self.index.get(&g) {
            return i;
        }
        let i = self.global.len() as u32;
        self.global.push(g);
        self.index.insert(g, i);
        self.level.push(level[g as usize]);
        self.cell.push(cell[g as usize]);
        self.arcs.push(HashMap::new());
        self.inn.push(Vec::new());
        i
    }

    /// Add an arc (u, w global) with a saved path (global numbers, stored in `paths`): kept if there is no arc u -> w
    /// or if it is cheaper (the first one wins a tie).
    fn add(&mut self, u: u32, w: u32, cost: f64, path: &[u32], paths: &mut Vec<u32>, level: &[u8], cell: &[u64]) {
        let (a, b) = (self.node(u, level, cell), self.node(w, level, cell));
        if self.arcs[a as usize].get(&b).is_some_and(|x| x.cost <= cost) {
            return;
        }
        let arc = Arc { cost, start: paths.len() as u32, len: path.len() as u32 };
        paths.extend_from_slice(path);
        if self.arcs[a as usize].insert(b, arc).is_none() {
            self.inn[b as usize].push(a);
        }
    }
}

/// The arcs of a bucket file, each pair once (the last row of a pair counts, as in duckOSM's router; an arc with no
/// cost is left out).
fn read_bucket(path: &Path) -> io::Result<Vec<(u32, u32, f64)>> {
    let mut b = Vec::new();
    if path.exists() {
        BufReader::new(File::open(path)?).read_to_end(&mut b)?;
    }
    let mut rows: Vec<(u32, u32, u64, f64)> = b
        .chunks_exact(24)
        .map(|c| {
            let u = u32::from_le_bytes(c[0..4].try_into().expect("4"));
            let w = u32::from_le_bytes(c[4..8].try_into().expect("4"));
            let cost = f64::from_le_bytes(c[8..16].try_into().expect("8"));
            let seq = u64::from_le_bytes(c[16..24].try_into().expect("8"));
            (u, w, seq, cost)
        })
        .collect();
    rows.sort_unstable_by_key(|r| (r.0, r.1, std::cmp::Reverse(r.2)));
    rows.dedup_by_key(|r| (r.0, r.1));
    Ok(rows.into_iter().filter(|r| !r.3.is_nan() && r.0 != r.1).map(|r| (r.0, r.1, r.3)).collect())
}

/// Build the tile files of `db` (mode `mode`) in `dir`, one zoom-8 tile at a time. The temporary files go to
/// `dir/.build` and are removed at the end.
pub fn build(db: &Path, mode: &str, finest: u8, dir: &Path) -> Result<Report, String> {
    let mut times = Vec::new();
    let t = std::time::Instant::now();
    std::fs::create_dir_all(dir).map_err(err)?;
    let tmp: PathBuf = dir.join(".build");
    std::fs::create_dir_all(&tmp).map_err(err)?;
    let con = load::open(db)?;
    let relation = load::relation(&con, mode)?;

    // 1. the nodes: the road edges, and the edges of the graph that are not in `edges` (level 0)
    let mut nodes: Vec<(i64, u8, u64)> = Vec::new();
    load::edge_levels(&con, mode, finest, |id, z, c| nodes.push((id, z, c)))?;
    {
        let mut st = con
            .prepare(&format!(
                "SELECT DISTINCT x FROM (SELECT from_edge AS x FROM {relation} UNION SELECT to_edge FROM {relation})
                 WHERE x NOT IN (SELECT edge_id FROM {mode}.edges)"
            ))
            .map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            nodes.push((r.get(0).map_err(err)?, 0, 0));
        }
    }
    nodes.sort_unstable_by_key(|x| x.0);
    nodes.dedup_by_key(|x| x.0);
    let key_of = |z: u8, c: u64| if z <= TOP { 0 } else { tile::parent(c, TOP + 1) };
    // the final node numbers: by file (the top first), then by level, then by cell
    let mut order: Vec<u32> = (0..nodes.len() as u32).collect();
    order.sort_unstable_by_key(|&i| {
        let (_, z, c) = nodes[i as usize];
        (key_of(z, c), z, c, i)
    });
    let n = nodes.len();
    let mut num = vec![0u32; n];
    for (k, &i) in order.iter().enumerate() {
        num[i as usize] = k as u32;
    }
    // by node number: level, cell, edge id
    let level: Vec<u8> = order.iter().map(|&i| nodes[i as usize].1).collect();
    let cell: Vec<u64> = order.iter().map(|&i| nodes[i as usize].2).collect();
    let edge: Vec<i64> = order.iter().map(|&i| nodes[i as usize].0).collect();
    let ids: Vec<i64> = nodes.iter().map(|x| x.0).collect();
    drop(nodes);
    let node_of = |id: i64| ids.binary_search(&id).ok().map(|i| num[i]);
    // the files: (first node, nodes, key)
    let mut files: Vec<(u32, u32, u64)> = Vec::new();
    for v in 0..n as u32 {
        let key = key_of(level[v as usize], cell[v as usize]);
        match files.last_mut() {
            Some(f) if f.2 == key => f.1 += 1,
            _ => files.push((v, 1, key)),
        }
    }
    if files.first().is_none_or(|f| f.2 != 0) {
        files.insert(0, (0, 0, 0));
    }
    let ntop = files[0].1 as usize;
    let key_of_node = |v: u32| key_of(level[v as usize], cell[v as usize]);
    times.push(("nodes", t.elapsed().as_secs_f64()));

    // 2. the arcs into bucket files
    let t = std::time::Instant::now();
    let bucket = |key: u64| tmp.join(format!("arcs_{key}.bin"));
    let mut writers: HashMap<u64, BufWriter<File>> = HashMap::new();
    let mut arcs = 0usize;
    {
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {relation}")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        let mut seq = 0u64;
        while let Some(r) = rows.next().map_err(err)? {
            let (a, b): (i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?);
            let c: Option<f64> = r.get(2).map_err(err)?;
            let (Some(u), Some(w)) = (node_of(a), node_of(b)) else { continue };
            let key = if level[u as usize] > TOP { key_of_node(u) } else { key_of_node(w) };
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
    }
    for (_, mut f) in writers.drain() {
        f.flush().map_err(err)?;
    }
    drop(con);
    times.push(("arcs into buckets", t.elapsed().as_secs_f64()));

    // 3. the fine steps, one zoom-8 tile at a time
    let t = std::time::Instant::now();
    let result = |key: u64| tmp.join(format!("result_{key}.bin"));
    // the shortcuts between two top nodes, from all tiles: the cheapest of each pair
    let mut top_short: HashMap<(u32, u32), (f64, Vec<u32>)> = HashMap::new();
    for &(first, k, key) in files.iter().filter(|f| f.2 != 0) {
        let mut g = Local::new();
        let mut paths: Vec<u32> = Vec::new();
        // the fine nodes of the tile first, so that each has a local number even with no arc
        for v in first..first + k {
            g.node(v, &level, &cell);
        }
        for (u, w, c) in read_bucket(&bucket(key)).map_err(err)? {
            g.add(u, w, c, &[], &mut paths, &level, &cell);
        }
        let m = g.global.len();
        let mut alive = vec![true; m];
        let mut st = Steps::new(finest, false);
        run_steps(&mut g.arcs, &mut g.inn, &mut alive, &g.level, &g.cell, (TOP + 1..=finest).rev(), &mut paths, &mut st);
        // paths hold local numbers: to global numbers
        let paths: Vec<u32> = paths.iter().map(|&x| g.global[x as usize]).collect();
        let mut out = BufWriter::new(File::create(result(key)).map_err(err)?);
        for (a, row) in g.arcs.iter().enumerate() {
            for (&b, arc) in row {
                let (u, w) = (g.global[a], g.global[b as usize]);
                let path = paths[arc.start as usize..(arc.start + arc.len) as usize].to_vec();
                if level[u as usize] > TOP || level[w as usize] > TOP {
                    write_row(&mut out, &Row { u, w, cost: arc.cost, path }).map_err(err)?;
                } else if arc.len > 0 {
                    // a shortcut between two top nodes
                    match top_short.get(&(u, w)) {
                        Some(x) if x.0 <= arc.cost => {}
                        _ => {
                            top_short.insert((u, w), (arc.cost, path));
                        }
                    }
                }
            }
        }
        out.flush().map_err(err)?;
    }
    times.push(("fine steps (zoom 20 to 8), tile by tile", t.elapsed().as_secs_f64()));

    // 4. the top: its arcs and the top shortcuts of all tiles; the steps of zoom 7 to 1
    let t = std::time::Instant::now();
    let mut top = Local::new();
    let mut top_paths: Vec<u32> = Vec::new();
    for v in 0..ntop as u32 {
        top.node(v, &level, &cell);
    }
    for (u, w, c) in read_bucket(&bucket(0)).map_err(err)? {
        top.add(u, w, c, &[], &mut top_paths, &level, &cell);
    }
    let mut short: Vec<((u32, u32), (f64, Vec<u32>))> = top_short.into_iter().collect();
    short.sort_unstable_by_key(|x| x.0);
    for ((u, w), (c, p)) in &short {
        top.add(*u, *w, *c, p, &mut top_paths, &level, &cell);
    }
    drop(short);
    // the top numbers are 0..ntop in both spaces, and the paths hold global numbers: no mapping needed
    let mut alive = vec![true; ntop];
    let mut st = Steps::new(TOP, false);
    run_steps(&mut top.arcs, &mut top.inn, &mut alive, &top.level, &top.cell, (1..=TOP).rev(), &mut top_paths, &mut st);
    // the search graphs of the top (a node of a lower level is higher; up: to the same or a higher node)
    let up_of = |u: u32, w: u32| level[w as usize] <= level[u as usize];
    let mut top_up: Vec<Vec<(f64, u32)>> = vec![Vec::new(); ntop];
    let mut top_down: Vec<Vec<(f64, u32)>> = vec![Vec::new(); ntop];
    for (u, row) in top.arcs.iter().enumerate() {
        for (&w, arc) in row {
            let u = u as u32;
            if up_of(u, w) {
                top_up[u as usize].push((arc.cost, w));
            }
            if up_of(w, u) {
                top_down[w as usize].push((arc.cost, u));
            }
        }
    }
    times.push(("top steps (zoom 7 to 1)", t.elapsed().as_secs_f64()));

    // 5. the files: each zoom-8 tile with arc reduction together with the top, then the top file
    let t = std::time::Instant::now();
    let flat = |rows: &[Vec<(f64, u32)>]| -> Fast {
        let mut off = vec![0u32];
        let mut arc = Vec::new();
        for r in rows {
            arc.extend_from_slice(r);
            off.push(arc.len() as u32);
        }
        Fast { off, arc }
    };
    let mut bytes = 0u64;
    // the arcs of the top file to fine nodes (from the tiles)
    let mut top_out: Vec<Vec<(u32, f64, Vec<u32>)>> = vec![Vec::new(); ntop];
    for &(first, k, key) in files.iter().filter(|f| f.2 != 0) {
        let rows = read_rows(&result(key)).map_err(err)?;
        let (first, k) = (first as usize, k as usize);
        // local space for the reduction: the top 0..ntop, then the nodes of the tile
        let loc = |g: u32| -> usize { if (g as usize) < ntop { g as usize } else { ntop + g as usize - first } };
        let glob = |l: usize| -> u32 { if l < ntop { l as u32 } else { (first + l - ntop) as u32 } };
        let mut up: Vec<Vec<(f64, u32)>> = top_up.iter().map(|r| r.iter().map(|&(c, w)| (c, w)).collect()).collect();
        let mut down: Vec<Vec<(f64, u32)>> = top_down.clone();
        up.resize(ntop + k, Vec::new());
        down.resize(ntop + k, Vec::new());
        let mut all: Vec<Vec<(u32, f64, Vec<u32>)>> = vec![Vec::new(); k];
        for r in rows {
            let (lu, lw) = (loc(r.u), loc(r.w));
            if lu >= ntop {
                if up_of(r.u, r.w) {
                    up[lu].push((r.cost, lw as u32));
                }
            }
            if lw >= ntop && up_of(r.w, r.u) {
                down[lw].push((r.cost, lu as u32));
            }
            if lu >= ntop {
                all[lu - ntop].push((r.w, r.cost, r.path));
            } else {
                top_out[r.u as usize].push((r.w, r.cost, r.path));
            }
        }
        // only the rows of the tile: the top is reduced once, for the top file
        let (up, down) = (reduce_from(flat(&up), ntop), reduce_from(flat(&down), ntop));
        let search = |f: &Fast| -> (Vec<u32>, Vec<SArc>) {
            let mut off = vec![0u32];
            let mut arcs = Vec::new();
            for l in ntop..ntop + k {
                arcs.extend(f.of(l as u32).iter().map(|&(cost, w)| SArc { cost, other: glob(w as usize), pad: 0 }));
                off.push(arcs.len() as u32);
            }
            (off, arcs)
        };
        let (up_off, up) = search(&up);
        let (down_off, down) = search(&down);
        let b = tile_file(first as u32, k, &level, &cell, &edge, (up_off, up), (down_off, down), all);
        std::fs::write(dir.join(disk::file_name(key)), &b).map_err(err)?;
        bytes += b.len() as u64;
    }
    // the top file
    {
        let up = reduce(flat(&top_up));
        let down = reduce(flat(&top_down));
        let search = |f: &Fast| -> (Vec<u32>, Vec<SArc>) {
            let mut off = vec![0u32];
            let mut arcs = Vec::new();
            for v in 0..ntop as u32 {
                arcs.extend(f.of(v).iter().map(|&(cost, w)| SArc { cost, other: w, pad: 0 }));
                off.push(arcs.len() as u32);
            }
            (off, arcs)
        };
        let (up_off, up) = search(&up);
        let (down_off, down) = search(&down);
        for (u, row) in top.arcs.iter().enumerate() {
            for (&w, arc) in row {
                top_out[u].push((w, arc.cost, top_paths[arc.start as usize..(arc.start + arc.len) as usize].to_vec()));
            }
        }
        let b = tile_file(0, ntop, &level, &cell, &edge, (up_off, up), (down_off, down), top_out);
        std::fs::write(dir.join(disk::file_name(0)), &b).map_err(err)?;
        bytes += b.len() as u64;
    }
    let mut pairs: Vec<(i64, u32)> = (0..n).map(|v| (edge[v], v as u32)).collect();
    pairs.sort_unstable();
    let b = disk::edges_bytes(&pairs);
    std::fs::write(dir.join("edges.idx"), &b).map_err(err)?;
    bytes += b.len() as u64;
    let b = disk::index_bytes(&files);
    std::fs::write(dir.join("index.bin"), &b).map_err(err)?;
    std::fs::remove_dir_all(&tmp).map_err(err)?;
    times.push(("files (arc reduction, tile by tile)", t.elapsed().as_secs_f64()));
    Ok(Report { nodes: n, arcs, tiles: files.len() - 1, top_nodes: ntop, bytes, times })
}

/// The bytes of one tile file: the nodes `first..first + k`, their search arcs and all their arcs (other node, cost,
/// saved inner nodes; sorted by the other node).
#[allow(clippy::too_many_arguments)]
fn tile_file(
    first: u32,
    k: usize,
    level: &[u8],
    cell: &[u64],
    edge: &[i64],
    up: (Vec<u32>, Vec<SArc>),
    down: (Vec<u32>, Vec<SArc>),
    mut all: Vec<Vec<(u32, f64, Vec<u32>)>>,
) -> Vec<u8> {
    let r = first as usize..first as usize + k;
    let (mut all_off, mut other, mut arc, mut paths) = (vec![0u32], Vec::new(), Vec::new(), Vec::new());
    for row in all.iter_mut() {
        row.sort_unstable_by_key(|x| x.0);
        for (w, cost, p) in row.iter() {
            other.push(*w);
            arc.push(AArc { cost: *cost, start: paths.len() as u32, len: p.len() as u32 });
            paths.extend_from_slice(p);
        }
        all_off.push(other.len() as u32);
    }
    disk::tile_bytes(
        first,
        k as u32,
        [
            disk::bytes_of(&level[r.clone()]),
            disk::bytes_of(&cell[r.clone()]),
            disk::bytes_of(&edge[r]),
            disk::bytes_of(&up.0),
            disk::bytes_of(&up.1),
            disk::bytes_of(&down.0),
            disk::bytes_of(&down.1),
            disk::bytes_of(&all_off),
            disk::bytes_of(&other),
            disk::bytes_of(&arc),
            disk::bytes_of(&paths),
        ],
    )
}
