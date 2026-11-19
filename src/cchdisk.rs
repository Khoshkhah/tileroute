//! The structure with no costs ([`crate::cch`]) and the costs of each metric, in files: one structure for all metrics,
//! and a small folder of numbers for each metric.
//!
//! | Path | Holds |
//! |---|---|
//! | `index.bin`, `edges.idx` | as in `crate::disk`: the files (first node, nodes, key), and the node of each duckOSM edge id |
//! | `structure/<file>` | for each node: its rank, its edge id, and its higher neighbours (node numbers, sorted) |
//! | `<metric>/<file>` | for each edge of `structure/<file>`, in the same order: the cost up and down, the middle node up and down, and one bit for each direction (does the search use this edge) |
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

/// Write the structure and the metrics. `ids`: the duckOSM edge id of each input node. The file of a node: the top
/// (key 0) if its level is `top` or lower, else its zoom-8 XYZ tile (`home`) or the cell of zoom `top + 1`.
#[allow(clippy::too_many_arguments)]
pub fn write(cch: &Cch, metrics: &[(&str, &Metric)], ids: &[i64], level: &[u8], cell: &[u64], home: Option<&[u64]>, top: u8, dir: &Path) -> io::Result<u64> {
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
    std::fs::create_dir_all(dir.join("structure"))?;
    for (name, _) in metrics {
        std::fs::create_dir_all(dir.join(name))?;
    }
    let mut bytes = 0u64;
    let to_num = |x: u32| if x == u32::MAX { u32::MAX } else { num[cch.order[x as usize] as usize] };
    for &(first, k, key) in &files {
        // for each node of the file: its higher neighbours (node numbers, sorted) and the edge of the structure of each
        let (mut rank, mut edge, mut off, mut other, mut src) = (Vec::new(), Vec::new(), vec![0u32], Vec::new(), Vec::new());
        for &v in &by[first as usize..(first + k) as usize] {
            let r = cch.rank[v as usize];
            rank.push(r);
            edge.push(ids[v as usize]);
            let mut row: Vec<(u32, usize)> = cch.row(r).map(|e| (to_num(cch.up[e]), e)).collect();
            row.sort_unstable();
            other.extend(row.iter().map(|x| x.0));
            src.extend(row.iter().map(|x| x.1));
            off.push(other.len() as u32);
        }
        let name = disk::file_name(key);
        let b = sections(STRUCTURE, first, k, &[disk::bytes_of(&rank), disk::bytes_of(&edge), disk::bytes_of(&off), disk::bytes_of(&other)]);
        std::fs::write(dir.join("structure").join(&name), &b)?;
        bytes += b.len() as u64;
        for (mname, m) in metrics {
            let up: Vec<f64> = src.iter().map(|&e| m.up[e]).collect();
            let down: Vec<f64> = src.iter().map(|&e| m.down[e]).collect();
            let via_up: Vec<u32> = src.iter().map(|&e| to_num(m.via_up[e])).collect();
            let via_down: Vec<u32> = src.iter().map(|&e| to_num(m.via_down[e])).collect();
            let bits = |keep: &[u64]| -> Vec<u64> {
                let mut out = vec![0u64; src.len().div_ceil(64)];
                for (j, &e) in src.iter().enumerate() {
                    if keep[e / 64] >> (e % 64) & 1 == 1 {
                        out[j / 64] |= 1 << (j % 64);
                    }
                }
                out
            };
            let (ku, kd) = (bits(&m.keep[0]), bits(&m.keep[1]));
            let b = sections(METRIC, first, k, &[disk::bytes_of(&up), disk::bytes_of(&down), disk::bytes_of(&via_up), disk::bytes_of(&via_down), disk::bytes_of(&ku), disk::bytes_of(&kd)]);
            std::fs::write(dir.join(mname).join(&name), &b)?;
            bytes += b.len() as u64;
        }
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
    fn via(&self, dir: usize) -> &[u32] {
        self.m.get(2 + dir)
    }
    fn keep(&self, dir: usize, j: usize) -> bool {
        self.m.get::<u64>(4 + dir)[j / 64] >> (j % 64) & 1 == 1
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

    /// The file of node `v` and the local number of `v` in it.
    fn at(&self, v: u32, read: &mut Vec<usize>) -> io::Result<(Arc<Tile>, usize)> {
        let f = self.files.partition_point(|x| x.0 <= v) - 1;
        let mut open = self.open.lock().expect("files");
        let t = match &open[f] {
            Some(t) => t.clone(),
            None => {
                let name = disk::file_name(self.files[f].2);
                let s = Part::open(&self.dir.join("structure").join(&name), STRUCTURE)?;
                let m = Part::open(&self.dir.join(&self.metric).join(&name), METRIC)?;
                let t = Arc::new(Tile { first: self.files[f].0, s, m });
                open[f] = Some(t.clone());
                t
            }
        };
        if !read.contains(&f) {
            read.push(f);
        }
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

    /// Add the nodes of the arc a → b after a: the edge is in the row of the lower rank; its middle node, if any.
    fn unpack(&self, a: u32, b: u32, path: &mut Vec<u32>, read: &mut Vec<usize>) -> io::Result<()> {
        let (ta, ia) = self.at(a, read)?;
        let (tb, ib) = self.at(b, read)?;
        let (lo_t, lo_i, hi, dir) = if ta.rank(ia) < tb.rank(ib) { (ta, ia, b, 0) } else { (tb, ib, a, 1) };
        let r = lo_t.row(lo_i);
        let k = lo_t.other()[r.clone()].binary_search(&hi).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("no edge {a} - {b} in the files")))?;
        let x = lo_t.via(dir)[r.start + k];
        if x == u32::MAX {
            path.push(b);
            Ok(())
        } else {
            self.unpack(a, x, path, read)?;
            self.unpack(x, b, path, read)
        }
    }
}
