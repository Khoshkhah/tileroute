//! The structure with no costs ([`crate::cch`]) and the costs of each metric, in files: one structure for all metrics,
//! and a small folder of numbers for each metric.
//!
//! | Path | Holds |
//! |---|---|
//! | `index.bin`, `edges.idx` | as in `crate::disk`: the files (first node, nodes, key), and the node of each duckOSM edge id |
//! | `structure/<file>` | for each node: its rank, its edge id, its items (time and length of the road edge), its higher neighbours and its lower neighbours (node numbers, sorted) |
//! | `<metric>/<file>` | for each edge of `structure/<file>` (the higher neighbours), in the same order: the cost up and down, and one bit for each direction (does the search use this edge) |
//!
//! The metric has no middle nodes: the unpack finds the middle node x of an edge a – b from the lower neighbours of
//! the lower end, with cost(a → x) + cost(x → b) = cost(a → b). On a tie between paths of equal cost, the path can
//! differ from the path in memory; the cost is the same.
//!
//! A node number is the same in all files: the nodes of one file are one range of numbers (by file, then by rank).
//! A new metric writes only its own folder.

use crate::cch::{Cch, Metric};
use crate::disk::{self, bad, cast, u32_at, u64_at, Key, Labels, Map};
use std::collections::BinaryHeap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const STRUCTURE: &[u8; 8] = b"TRCCHS01";
const METRIC: &[u8; 8] = b"TRCCHM01";

/// magic, first node, nodes, number of sections, then (offset, bytes) of each section; each section at a multiple of 8
fn sections(magic: &[u8; 8], first: u32, k: u32, parts: &[&[u8]]) -> Vec<u8> {
    let head = 24 + 16 * parts.len();
    let mut out = Vec::with_capacity(head + parts.iter().map(|p| p.len() + 8).sum::<usize>());
    out.extend_from_slice(magic);
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&k.to_le_bytes());
    out.extend_from_slice(&(parts.len() as u64).to_le_bytes());
    out.resize(head, 0);
    for (i, p) in parts.iter().enumerate() {
        let at = out.len();
        out[24 + 16 * i..32 + 16 * i].copy_from_slice(&(at as u64).to_le_bytes());
        out[32 + 16 * i..40 + 16 * i].copy_from_slice(&(p.len() as u64).to_le_bytes());
        out.extend_from_slice(p);
        out.resize(out.len().next_multiple_of(8), 0);
    }
    out
}

/// The data of one file: for each node (by node number) its rank, edge id, items, higher neighbours (`off`, `other`,
/// sorted) and lower neighbours (`doff`, `down`, sorted); for each metric the costs up and down and the mask bits of
/// each edge of `other`.
pub(crate) struct FileData {
    pub rank: Vec<u32>,
    pub edge: Vec<i64>,
    pub item: Vec<f32>,
    pub off: Vec<u32>,
    pub other: Vec<u32>,
    pub doff: Vec<u32>,
    pub down: Vec<u32>,
    pub metrics: Vec<(Vec<f64>, Vec<f64>, Vec<u64>, Vec<u64>)>,
}

/// Write `structure/<file>` and `<metric>/<file>` of one file; returns the bytes.
pub(crate) fn write_file(dir: &Path, names: &[&str], key: u64, first: u32, d: &FileData) -> io::Result<u64> {
    let k = d.rank.len() as u32;
    let name = disk::file_name(key);
    let b = sections(
        STRUCTURE,
        first,
        k,
        &[disk::bytes_of(&d.rank), disk::bytes_of(&d.edge), disk::bytes_of(&d.off), disk::bytes_of(&d.other), disk::bytes_of(&d.item), disk::bytes_of(&d.doff), disk::bytes_of(&d.down)],
    );
    std::fs::create_dir_all(dir.join("structure"))?;
    std::fs::write(dir.join("structure").join(&name), &b)?;
    let mut bytes = b.len() as u64;
    for (mname, (up, down, ku, kd)) in names.iter().zip(&d.metrics) {
        let b = sections(METRIC, first, k, &[disk::bytes_of(up), disk::bytes_of(down), disk::bytes_of(ku), disk::bytes_of(kd)]);
        std::fs::create_dir_all(dir.join(mname))?;
        std::fs::write(dir.join(mname).join(&name), &b)?;
        bytes += b.len() as u64;
    }
    Ok(bytes)
}

/// Write the structure and the metrics. `ids`: the duckOSM edge id of each input node. The file of a node: the top
/// (key 0) if its level is `top` or lower, else its zoom-8 XYZ tile (`home`) or the cell of zoom `top + 1`.
#[allow(clippy::too_many_arguments)]
/// `items`: the time and the length of each input node (its road edge).
pub fn write(cch: &Cch, metrics: &[(&str, &Metric)], ids: &[i64], items: &[[f32; 2]], level: &[u8], cell: &[u64], home: Option<&[u64]>, top: u8, dir: &Path) -> io::Result<u64> {
    let n = ids.len();
    let key_of = |v: usize| -> u64 {
        if level[v] <= top {
            0
        } else if let Some(h) = home {
            h[v]
        } else {
            crate::tile::parent(cell[v], top + 1)
        }
    };
    let mut by: Vec<u32> = (0..n as u32).collect();
    by.sort_unstable_by_key(|&v| (key_of(v as usize), cch.rank[v as usize]));
    let mut num = vec![0u32; n];
    for (i, &v) in by.iter().enumerate() {
        num[v as usize] = i as u32;
    }
    let mut files: Vec<(u32, u32, u64)> = Vec::new();
    for (i, &v) in by.iter().enumerate() {
        let key = key_of(v as usize);
        match files.last_mut() {
            Some(f) if f.2 == key => f.1 += 1,
            _ => files.push((i as u32, 1, key)),
        }
    }
    if files.first().is_none_or(|f| f.2 != 0) {
        files.insert(0, (0, 0, 0));
    }
    let mut bytes = 0u64;
    let to_num = |x: u32| num[cch.order[x as usize] as usize];
    // the lower neighbours of each node (node numbers)
    let mut lower: Vec<Vec<u32>> = vec![Vec::new(); n];
    for r in 0..n as u32 {
        let lo = to_num(r);
        for e in cch.row(r) {
            lower[to_num(cch.up[e]) as usize].push(lo);
        }
    }
    for &(first, k, key) in &files {
        // for each node of the file: its higher neighbours (node numbers, sorted) and the edge of the structure of each
        let (mut rank, mut edge, mut off, mut other, mut src): (Vec<u32>, Vec<i64>, Vec<u32>, Vec<u32>, Vec<usize>) = (Vec::new(), Vec::new(), vec![0u32], Vec::new(), Vec::new());
        let (mut item, mut doff, mut down) = (Vec::new(), vec![0u32], Vec::new());
        for &v in &by[first as usize..(first + k) as usize] {
            let r = cch.rank[v as usize];
            rank.push(r);
            edge.push(ids[v as usize]);
            item.extend_from_slice(&items[v as usize]);
            let lw = &mut lower[num[v as usize] as usize];
            lw.sort_unstable();
            down.extend_from_slice(lw);
            doff.push(down.len() as u32);
            let mut row: Vec<(u32, usize)> = cch.row(r).map(|e| (to_num(cch.up[e]), e)).collect();
            row.sort_unstable();
            other.extend(row.iter().map(|x| x.0));
            src.extend(row.iter().map(|x| x.1));
            off.push(other.len() as u32);
        }
        let mut ms = Vec::new();
        for (_, m) in metrics {
            let up: Vec<f64> = src.iter().map(|&e| m.up[e]).collect();
            let down: Vec<f64> = src.iter().map(|&e| m.down[e]).collect();
            let bits = |keep: &[u64]| -> Vec<u64> {
                let mut out = vec![0u64; src.len().div_ceil(64)];
                for (j, &e) in src.iter().enumerate() {
                    if keep[e / 64] >> (e % 64) & 1 == 1 {
                        out[j / 64] |= 1 << (j % 64);
                    }
                }
                out
            };
            ms.push((up, down, bits(&m.keep[0]), bits(&m.keep[1])));
        }
        let names: Vec<&str> = metrics.iter().map(|m| m.0).collect();
        bytes += write_file(dir, &names, key, first, &FileData { rank, edge, item, off, other, doff, down, metrics: ms })?;
    }
    let mut pairs: Vec<(i64, u32)> = (0..n).map(|v| (ids[v], num[v])).collect();
    pairs.sort_unstable();
    let b = disk::edges_bytes(&pairs);
    std::fs::write(dir.join("edges.idx"), &b)?;
    bytes += b.len() as u64;
    std::fs::write(dir.join("index.bin"), disk::index_bytes(&files))?;
    Ok(bytes)
}

/// One mapped file of sections.
struct Part {
    map: Map,
    sec: Vec<(usize, usize)>,
}

impl Part {
    fn open(path: &Path, magic: &[u8; 8]) -> io::Result<Part> {
        let map = Map::open(path)?;
        let b = map.bytes();
        if b.len() < 24 || &b[..8] != magic {
            return Err(bad(path, "not a tileroute structure or metric file"));
        }
        let count = u64_at(b, 16) as usize;
        let sec = (0..count).map(|i| (u64_at(b, 24 + 16 * i) as usize, u64_at(b, 32 + 16 * i) as usize)).collect::<Vec<_>>();
        if sec.iter().any(|&(o, l)| o % 8 != 0 || o + l > b.len()) {
            return Err(bad(path, "a section is out of the file"));
        }
        Ok(Part { map, sec })
    }

    fn get<T: disk::Pod>(&self, i: usize) -> &[T] {
        let (o, l) = self.sec[i];
        cast(&self.map.bytes()[o..o + l])
    }
}

/// The structure and the metric of one file.
struct Tile {
    first: u32,
    s: Part,
    m: Part,
}

impl Tile {
    fn rank(&self, i: usize) -> u32 {
        self.s.get::<u32>(0)[i]
    }
    fn edge(&self, i: usize) -> i64 {
        self.s.get::<i64>(1)[i]
    }
    fn row(&self, i: usize) -> std::ops::Range<usize> {
        let o: &[u32] = self.s.get(2);
        o[i] as usize..o[i + 1] as usize
    }
    fn other(&self) -> &[u32] {
        self.s.get(3)
    }
    fn cost(&self, dir: usize) -> &[f64] {
        self.m.get(dir)
    }
    fn keep(&self, dir: usize, j: usize) -> bool {
        self.m.get::<u64>(2 + dir)[j / 64] >> (j % 64) & 1 == 1
    }
    /// time and length of node i
    pub(crate) fn items(&self, i: usize) -> [f32; 2] {
        let x: &[f32] = self.s.get(4);
        [x[2 * i], x[2 * i + 1]]
    }
    fn lower(&self, i: usize) -> &[u32] {
        let o: &[u32] = self.s.get(5);
        &self.s.get::<u32>(6)[o[i] as usize..o[i + 1] as usize]
    }
    /// The cost of the edge from node i (local) to its higher neighbour w, in direction `dir` (0: i → w).
    fn cost_to(&self, i: usize, w: u32, dir: usize) -> Option<f64> {
        let r = self.row(i);
        self.other()[r.clone()].binary_search(&w).ok().map(|k| self.cost(dir)[r.start + k])
    }
}

/// A folder written by [`write`], with one metric.
pub struct Store {
    dir: PathBuf,
    metric: String,
    files: Vec<(u32, u32, u64)>,
    edges: Map,
    n_edges: usize,
    // ponytail: all opened files stay open; an LRU as in `disk::Store` when an area has thousands of files
    open: Mutex<Vec<Option<Arc<Tile>>>>,
}

/// A route: its cost and its duckOSM edge ids (None: no path).
pub type Route = Option<(f64, Vec<i64>)>;

/// The files that one query uses (file number, file).
type Open = Vec<(usize, Arc<Tile>)>;

impl Store {
    pub fn open(dir: &Path, metric: &str) -> io::Result<Store> {
        let path = dir.join("index.bin");
        let b = std::fs::read(&path)?;
        if b.len() < 16 || &b[..8] != disk::INDEX_MAGIC {
            return Err(bad(&path, "not a tileroute index"));
        }
        let k = u64_at(&b, 8) as usize;
        let files: Vec<(u32, u32, u64)> = (0..k).map(|i| (u32_at(&b, 16 + i * 16), u32_at(&b, 20 + i * 16), u64_at(&b, 24 + i * 16))).collect();
        let edges = Map::open(&dir.join("edges.idx"))?;
        let n_edges = u64_at(edges.bytes(), 8) as usize;
        if !dir.join(metric).is_dir() {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("no metric {metric} in {}", dir.display())));
        }
        Ok(Store { dir: dir.into(), metric: metric.into(), open: Mutex::new(vec![None; files.len()]), files, edges, n_edges })
    }

    pub fn n_files(&self) -> usize {
        self.files.len()
    }

    fn node(&self, edge: i64) -> Option<u32> {
        let b = self.edges.bytes();
        let ids: &[i64] = cast(&b[16..16 + self.n_edges * 8]);
        let nodes: &[u32] = cast(&b[16 + self.n_edges * 8..16 + self.n_edges * 12]);
        ids.binary_search(&edge).ok().map(|i| nodes[i])
    }

    /// The file of node `v` and the local number of `v` in it: from the files of this query, or from the store.
    fn at(&self, v: u32, read: &mut Open) -> io::Result<(Arc<Tile>, usize)> {
        let f = self.files.partition_point(|x| x.0 <= v) - 1;
        if let Some((_, t)) = read.iter().find(|e| e.0 == f) {
            return Ok((t.clone(), (v - t.first) as usize));
        }
        let t = {
            let mut open = self.open.lock().expect("files");
            match &open[f] {
                Some(t) => t.clone(),
                None => {
                    let name = disk::file_name(self.files[f].2);
                    let s = Part::open(&self.dir.join("structure").join(&name), STRUCTURE)?;
                    let m = Part::open(&self.dir.join(&self.metric).join(&name), METRIC)?;
                    let t = Arc::new(Tile { first: self.files[f].0, s, m });
                    open[f] = Some(t.clone());
                    t
                }
            }
        };
        read.push((f, t.clone()));
        let i = (v - t.first) as usize;
        Ok((t, i))
    }

    /// The shortest path between two duckOSM edges with this metric, and the number of files that it read.
    pub fn route(&self, from: i64, to: i64) -> io::Result<(Route, usize)> {
        let (Some(s), Some(t)) = (self.node(from), self.node(to)) else {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("edge {from} or {to} is not in the files")));
        };
        let mut read = Vec::new();
        if s == t {
            let (tile, i) = self.at(s, &mut read)?;
            return Ok((Some((0.0, vec![tile.edge(i)])), read.len()));
        }
        let mut label: [Labels; 2] = Default::default();
        let mut heap: [BinaryHeap<Key>; 2] = [BinaryHeap::new(), BinaryHeap::new()];
        label[0].insert(s, (0.0, u32::MAX));
        label[1].insert(t, (0.0, u32::MAX));
        heap[0].push(Key(0.0, s));
        heap[1].push(Key(0.0, t));
        let (mut best, mut meet) = (f64::INFINITY, u32::MAX);
        loop {
            let top = |h: &BinaryHeap<Key>| h.peek().map_or(f64::INFINITY, |k| k.0);
            let (f, b) = (top(&heap[0]), top(&heap[1]));
            if f.min(b) >= best || (f.is_infinite() && b.is_infinite()) {
                break;
            }
            let dir = if f <= b { 0 } else { 1 };
            let Key(d, v) = heap[dir].pop().expect("not empty");
            if d > label[dir][&v].0 {
                continue;
            }
            if let Some(&(e, _)) = label[1 - dir].get(&v) {
                if d + e < best {
                    best = d + e;
                    meet = v;
                }
            }
            let (tile, i) = self.at(v, &mut read)?;
            let (other, cost) = (tile.other(), tile.cost(dir));
            for j in tile.row(i) {
                if !tile.keep(dir, j) {
                    continue;
                }
                let (w, nd) = (other[j], d + cost[j]);
                if label[dir].get(&w).is_none_or(|&(e, _)| nd < e) {
                    label[dir].insert(w, (nd, v));
                    heap[dir].push(Key(nd, w));
                }
            }
        }
        if meet == u32::MAX {
            return Ok((None, read.len()));
        }
        let chain = |dir: usize| {
            let mut c = vec![meet];
            while let Some(&(_, p)) = label[dir].get(c.last().expect("not empty")) {
                if p == u32::MAX {
                    break;
                }
                c.push(p);
            }
            c
        };
        let mut nodes = chain(0);
        nodes.reverse();
        nodes.extend(chain(1).into_iter().skip(1));
        let mut path = vec![s];
        for w in nodes.windows(2) {
            self.unpack(w[0], w[1], &mut path, &mut read)?;
        }
        let ids = path.into_iter().map(|v| self.at(v, &mut read).map(|(t, i)| t.edge(i))).collect::<io::Result<_>>()?;
        Ok((Some((best, ids)), read.len()))
    }

    /// The cost of the arc a → b (one of the two is the higher neighbour of the other).
    fn arc(&self, a: u32, b: u32, read: &mut Open) -> io::Result<f64> {
        let (ta, ia) = self.at(a, read)?;
        let (tb, ib) = self.at(b, read)?;
        let c = if ta.rank(ia) < tb.rank(ib) { ta.cost_to(ia, b, 0) } else { tb.cost_to(ib, a, 1) };
        c.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("no edge {a} - {b} in the files")))
    }

    /// Add the nodes of the arc a → b after a. The middle node x is a lower neighbour of the lower end with an edge to
    /// the higher end and cost(a → x) + cost(x → b) = cost(a → b); with no such x the arc is an input arc.
    fn unpack(&self, a: u32, b: u32, path: &mut Vec<u32>, read: &mut Open) -> io::Result<()> {
        let c = self.arc(a, b, read)?;
        let (ta, ia) = self.at(a, read)?;
        let (tb, ib) = self.at(b, read)?;
        // the common lower neighbours of a and b (both lists sorted)
        let (la, lb) = (ta.lower(ia), tb.lower(ib));
        let (mut i, mut j) = (0, 0);
        while i < la.len() && j < lb.len() {
            match la[i].cmp(&lb[j]) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    let x = la[i];
                    let (tx, ix) = self.at(x, read)?;
                    // x is lower than a and b: both edges are in the row of x
                    if let (Some(ax), Some(xb)) = (tx.cost_to(ix, a, 1), tx.cost_to(ix, b, 0)) {
                        if ax + xb == c {
                            self.unpack(a, x, path, read)?;
                            return self.unpack(x, b, path, read);
                        }
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        path.push(b);
        Ok(())
    }

    /// The items (time, length) of a route: the sum over its road edges except the last (an arc costs its first road
    /// edge, as in duckOSM).
    pub fn items(&self, route: &[i64]) -> io::Result<[f64; 2]> {
        let mut read = Vec::new();
        let mut sum = [0.0f64; 2];
        for &id in &route[..route.len().saturating_sub(1)] {
            let v = self.node(id).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("edge {id}")))?;
            let (t, i) = self.at(v, &mut read)?;
            let x = t.items(i);
            sum[0] += x[0] as f64;
            sum[1] += x[1] as f64;
        }
        Ok(sum)
    }
}
