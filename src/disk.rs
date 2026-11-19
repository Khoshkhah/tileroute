//! The hierarchy in tile files on disk, and the query that reads only the tiles that it needs.
//!
//! | File | Holds |
//! |---|---|
//! | `index.bin` | for each tile file: its first node, its number of nodes, its tile key (the top file first) |
//! | `top.trt` | the nodes of level 7 and lower (the cell is a tile of zoom 7 or coarser) |
//! | `8_<x>_<y>.trt` | the nodes whose cell is in this zoom-8 tile |
//! | `edges.idx` | the duckOSM `edge_id`s, sorted, and the node of each |
//!
//! A node number is the same in all files: the nodes of one file are one range of numbers, so the file of a node is a
//! binary search in `index.bin`. All numbers are little-endian and each section starts at a multiple of 8 bytes, so the
//! reader uses the memory map with no decode step.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::fs::File;
use std::hash::{BuildHasherDefault, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as Mem};
use std::sync::{Arc, Mutex};

const _: () = assert!(cfg!(target_endian = "little"), "the files are little-endian and read with no decode step");

/// The finest zoom of the top file; a node of a finer level goes to the file of its zoom-8 tile.
pub const TOP: u8 = 7;
pub const TILE_MAGIC: &[u8; 8] = b"TRTILE01";
pub const INDEX_MAGIC: &[u8; 8] = b"TRINDX01";
pub const EDGES_MAGIC: &[u8; 8] = b"TREDGE01";

/// The sections of a tile file, in this order. Per node: level u8, cell u64, edge_id i64; the forward-search arcs
/// (offsets u32[k+1], `SArc`), the backward-search arcs (the same), all arcs (offsets, other node u32, `AArc`) and the
/// inner nodes of the shortcuts (u32).
pub const LEVEL: usize = 0;
pub const CELL: usize = 1;
pub const EDGE: usize = 2;
pub const UP_OFF: usize = 3;
pub const UP: usize = 4;
pub const DOWN_OFF: usize = 5;
pub const DOWN: usize = 6;
pub const ALL_OFF: usize = 7;
pub const ALL_OTHER: usize = 8;
pub const ALL_ARC: usize = 9;
pub const PATHS: usize = 10;
pub const SECTIONS: usize = 11;
/// magic, first node u32, nodes u32, then (offset u64, bytes u64) for each section
const HEADER: usize = 16 + SECTIONS * 16;

/// An arc of a search: its cost and the other node.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SArc {
    pub cost: f64,
    pub other: u32,
    pub pad: u32,
}

/// An arc in the list of all arcs: its cost, and its inner nodes `PATHS[start..start + len]` of the same file.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct AArc {
    pub cost: f64,
    pub start: u32,
    pub len: u32,
}

/// A type that is only bytes, with no padding that matters and no pointers.
///
/// # Safety
/// Implement it only for `repr(C)` types of plain numbers.
pub unsafe trait Pod: Copy {}
unsafe impl Pod for u8 {}
unsafe impl Pod for u32 {}
unsafe impl Pod for u64 {}
unsafe impl Pod for i64 {}
unsafe impl Pod for f64 {}
unsafe impl Pod for f32 {}
unsafe impl Pod for SArc {}
unsafe impl Pod for AArc {}

pub fn bytes_of<T: Pod>(xs: &[T]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(xs.as_ptr() as *const u8, std::mem::size_of_val(xs)) }
}

pub(crate) fn cast<T: Pod>(b: &[u8]) -> &[T] {
    let size = std::mem::size_of::<T>();
    assert!((b.as_ptr() as usize).is_multiple_of(std::mem::align_of::<T>()) && b.len().is_multiple_of(size), "a section is not aligned");
    unsafe { std::slice::from_raw_parts(b.as_ptr() as *const T, b.len() / size) }
}

fn pad8(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(8), 0);
}

/// The bytes of one tile file.
pub fn tile_bytes(first: u32, k: u32, sections: [&[u8]; SECTIONS]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + sections.iter().map(|s| s.len() + 8).sum::<usize>());
    out.extend_from_slice(TILE_MAGIC);
    out.extend_from_slice(&first.to_le_bytes());
    out.extend_from_slice(&k.to_le_bytes());
    out.resize(HEADER, 0);
    for (i, s) in sections.iter().enumerate() {
        let at = out.len();
        out[16 + i * 16..24 + i * 16].copy_from_slice(&(at as u64).to_le_bytes());
        out[24 + i * 16..32 + i * 16].copy_from_slice(&(s.len() as u64).to_le_bytes());
        out.extend_from_slice(s);
        pad8(&mut out);
    }
    out
}

/// The bytes of `index.bin`: (first node, nodes, tile key) for each file.
pub fn index_bytes(files: &[(u32, u32, u64)]) -> Vec<u8> {
    let mut out = INDEX_MAGIC.to_vec();
    out.extend_from_slice(&(files.len() as u64).to_le_bytes());
    for &(first, k, key) in files {
        out.extend_from_slice(&first.to_le_bytes());
        out.extend_from_slice(&k.to_le_bytes());
        out.extend_from_slice(&key.to_le_bytes());
    }
    out
}

/// The bytes of `edges.idx`: the edge ids, sorted, then the node of each. `pairs`: (edge id, node), sorted.
pub fn edges_bytes(pairs: &[(i64, u32)]) -> Vec<u8> {
    let mut out = EDGES_MAGIC.to_vec();
    out.extend_from_slice(&(pairs.len() as u64).to_le_bytes());
    pairs.iter().for_each(|p| out.extend_from_slice(&p.0.to_le_bytes()));
    pairs.iter().for_each(|p| out.extend_from_slice(&p.1.to_le_bytes()));
    out
}

/// The name of the file of a tile key (key 0: the top file).
pub fn file_name(key: u64) -> String {
    if key == 0 {
        return "top.trt".into();
    }
    let (x, y) = crate::tile::xy(key);
    format!("{}_{x}_{y}.trt", crate::tile::zoom(key))
}

/// A read-only memory map of a file.
pub(crate) struct Map {
    ptr: *mut libc::c_void,
    len: usize,
}

unsafe impl Send for Map {}
unsafe impl Sync for Map {}

impl Map {
    pub(crate) fn open(path: &Path) -> io::Result<Map> {
        use std::os::fd::AsRawFd;
        let f = File::open(path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let len = f.metadata()?.len() as usize;
        if len == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}: empty", path.display())));
        }
        let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
        if ptr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        Ok(Map { ptr, len })
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for Map {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr, self.len) };
    }
}

pub(crate) fn bad(path: &Path, what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{}: {what}", path.display()))
}

pub(crate) fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}

/// One memory-mapped tile file.
pub struct Tile {
    map: Map,
    first: u32,
    sec: [(usize, usize); SECTIONS],
}

impl Tile {
    fn open(path: &Path) -> io::Result<Tile> {
        let map = Map::open(path)?;
        let b = map.bytes();
        if b.len() < HEADER || &b[..8] != TILE_MAGIC {
            return Err(bad(path, "not a tileroute tile file"));
        }
        let mut sec = [(0, 0); SECTIONS];
        for (i, s) in sec.iter_mut().enumerate() {
            *s = (u64_at(b, 16 + i * 16) as usize, u64_at(b, 24 + i * 16) as usize);
            if s.0 % 8 != 0 || s.0 + s.1 > b.len() {
                return Err(bad(path, "a section is out of the file"));
            }
        }
        let first = u32_at(b, 8);
        Ok(Tile { map, first, sec })
    }

    fn get<T: Pod>(&self, s: usize) -> &[T] {
        let (o, l) = self.sec[s];
        cast(&self.map.bytes()[o..o + l])
    }

    /// The range of local arc numbers of local node `i` in the offsets section `off`.
    fn range(&self, off: usize, i: usize) -> std::ops::Range<usize> {
        let o: &[u32] = self.get(off);
        o[i] as usize..o[i + 1] as usize
    }
}

/// A route: its cost and its duckOSM edge ids (None: no path).
pub type Route = Option<(f64, Vec<i64>)>;

/// What one query read.
#[derive(Clone, Copy, Default, Debug)]
pub struct Stats {
    /// the tile files that the query used
    pub tiles: usize,
    /// the bytes of the files that the query read (headers not counted)
    pub bytes: usize,
}

/// A fast hash for node numbers (the labels of a query).
#[derive(Default)]
pub(crate) struct NodeHash(u64);
impl Hasher for NodeHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, b: &[u8]) {
        b.iter().for_each(|&x| self.write_u64(x as u64));
    }
    fn write_u32(&mut self, x: u32) {
        self.write_u64(x as u64);
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}
pub(crate) type Labels = HashMap<u32, (f64, u32), BuildHasherDefault<NodeHash>>;

/// A heap entry, the smallest distance first; a tie takes the smaller node (as `hier`).
#[derive(PartialEq)]
pub(crate) struct Key(pub(crate) f64, pub(crate) u32);
impl Eq for Key {}
impl Ord for Key {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
    }
}
impl PartialOrd for Key {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// The folder of tile files, and a small cache of open (memory-mapped) tiles.
pub struct Store {
    dir: PathBuf,
    /// (first node, nodes, tile key) for each file, by first node
    files: Vec<(u32, u32, u64)>,
    edges: Map,
    n_edges: usize,
    // ponytail: an LRU by linear scan; a query takes each tile from it once, so the scan is cheap below some hundred tiles
    cache: Mutex<Vec<(usize, Arc<Tile>, u64)>>,
    clock: AtomicU64,
    cap: usize,
    /// the files mapped since the start (a file mapped again after the cache dropped it counts again)
    pub mapped: AtomicU64,
}

impl Store {
    /// Open a folder that `tileroute build` wrote. `cap`: the open tiles that the cache keeps.
    pub fn open(dir: &Path, cap: usize) -> io::Result<Store> {
        let path = dir.join("index.bin");
        let b = std::fs::read(&path).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        if b.len() < 16 || &b[..8] != INDEX_MAGIC {
            return Err(bad(&path, "not a tileroute index"));
        }
        let k = u64_at(&b, 8) as usize;
        if b.len() != 16 + k * 16 {
            return Err(bad(&path, "wrong size"));
        }
        let files = (0..k).map(|i| (u32_at(&b, 16 + i * 16), u32_at(&b, 20 + i * 16), u64_at(&b, 24 + i * 16))).collect();
        let epath = dir.join("edges.idx");
        let edges = Map::open(&epath)?;
        let eb = edges.bytes();
        if eb.len() < 16 || &eb[..8] != EDGES_MAGIC {
            return Err(bad(&epath, "not a tileroute edge index"));
        }
        let n_edges = u64_at(eb, 8) as usize;
        if eb.len() < 16 + n_edges * 12 {
            return Err(bad(&epath, "too short"));
        }
        Ok(Store { dir: dir.into(), files, edges, n_edges, cache: Mutex::new(Vec::new()), clock: AtomicU64::new(0), cap: cap.max(1), mapped: AtomicU64::new(0) })
    }

    pub fn n_files(&self) -> usize {
        self.files.len()
    }

    /// The node of a duckOSM edge id, and the bytes that the binary search read.
    pub fn node(&self, edge: i64) -> (Option<u32>, usize) {
        let b = self.edges.bytes();
        let ids: &[i64] = cast(&b[16..16 + self.n_edges * 8]);
        let nodes: &[u32] = cast(&b[16 + self.n_edges * 8..16 + self.n_edges * 12]);
        let steps = (usize::BITS - self.n_edges.leading_zeros()) as usize;
        (ids.binary_search(&edge).ok().map(|i| nodes[i]), steps * 8 + 4)
    }

    /// The tile of file number `f`: from the cache, or mapped now.
    fn tile(&self, f: usize) -> io::Result<Arc<Tile>> {
        let now = self.clock.fetch_add(1, Mem::Relaxed);
        let mut c = self.cache.lock().expect("cache");
        if let Some(e) = c.iter_mut().find(|e| e.0 == f) {
            e.2 = now;
            return Ok(e.1.clone());
        }
        let t = Arc::new(Tile::open(&self.dir.join(file_name(self.files[f].2)))?);
        self.mapped.fetch_add(1, Mem::Relaxed);
        if c.len() >= self.cap {
            let old = c.iter().enumerate().min_by_key(|(_, e)| e.2).map(|(i, _)| i).expect("not empty");
            c.swap_remove(old);
        }
        c.push((f, t.clone(), now));
        Ok(t)
    }

    /// The shortest path between two duckOSM edges: its cost and its edge ids (None: no path), and what the query read.
    pub fn route(&self, from: i64, to: i64) -> io::Result<(Route, Stats)> {
        let mut q = Query { store: self, open: Vec::new(), stats: Stats::default() };
        let (s, bs) = self.node(from);
        let (t, bt) = self.node(to);
        q.stats.bytes += bs + bt;
        let (Some(s), Some(t)) = (s, t) else {
            return Err(io::Error::new(io::ErrorKind::NotFound, format!("edge {from} or {to} is not in the files")));
        };
        let r = q.route(s, t)?;
        Ok((r, q.stats))
    }
}

/// One query: the tiles that it opened, and what it read.
struct Query<'a> {
    store: &'a Store,
    open: Vec<(usize, Arc<Tile>)>,
    stats: Stats,
}

impl Query<'_> {
    /// The tile of node `v`, and the local number of `v` in it.
    fn at(&mut self, v: u32) -> io::Result<(Arc<Tile>, usize)> {
        let f = self.store.files.partition_point(|x| x.0 <= v) - 1;
        let t = match self.open.iter().find(|e| e.0 == f) {
            Some(e) => e.1.clone(),
            None => {
                let t = self.store.tile(f)?;
                self.open.push((f, t.clone()));
                self.stats.tiles += 1;
                t
            }
        };
        let i = (v - t.first) as usize;
        Ok((t, i))
    }

    /// The bidirectional search of `hier::Hierarchy::search`, on the files.
    fn route(&mut self, s: u32, t: u32) -> io::Result<Route> {
        if s == t {
            return Ok(Some((0.0, vec![self.edge(s)?])));
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
            let (tile, i) = self.at(v)?;
            let (off, sec) = if dir == 0 { (UP_OFF, UP) } else { (DOWN_OFF, DOWN) };
            let arcs = &tile.get::<SArc>(sec)[tile.range(off, i)];
            self.stats.bytes += 8 + arcs.len() * 16;
            for a in arcs {
                let nd = d + a.cost;
                if label[dir].get(&a.other).is_none_or(|&(e, _)| nd < e) {
                    label[dir].insert(a.other, (nd, v));
                    heap[dir].push(Key(nd, a.other));
                }
            }
        }
        if meet == u32::MAX {
            return Ok(None);
        }
        // the chain of each search, then each arc unpacked
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
            self.unpack(w[0], w[1], &mut path)?;
        }
        let ids = path.into_iter().map(|v| self.edge(v)).collect::<io::Result<_>>()?;
        Ok(Some((best, ids)))
    }

    fn edge(&mut self, v: u32) -> io::Result<i64> {
        let (tile, i) = self.at(v)?;
        self.stats.bytes += 8;
        Ok(tile.get::<i64>(EDGE)[i])
    }

    /// Add the nodes of the arc `a -> b` after `a` (as `hier::Hierarchy::unpack`).
    fn unpack(&mut self, a: u32, b: u32, path: &mut Vec<u32>) -> io::Result<()> {
        let (tile, i) = self.at(a)?;
        let r = tile.range(ALL_OFF, i);
        let other = &tile.get::<u32>(ALL_OTHER)[r.clone()];
        let k = other.binary_search(&b).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("no arc {a} -> {b} in the files")))?;
        let arc = tile.get::<AArc>(ALL_ARC)[r.start + k];
        let inner = &tile.get::<u32>(PATHS)[arc.start as usize..(arc.start + arc.len) as usize];
        self.stats.bytes += 8 + other.len() * 4 + 16 + inner.len() * 4;
        let mut prev = a;
        for &x in inner {
            self.unpack(prev, x, path)?;
            prev = x;
        }
        if arc.len == 0 {
            path.push(b);
            Ok(())
        } else {
            self.unpack(prev, b, path)
        }
    }
}

/// The peak memory of this process (resident set), in bytes.
pub fn peak_rss() -> u64 {
    let mut u: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
    // Linux gives kilobytes, macOS bytes
    if cfg!(target_os = "macos") { u.ru_maxrss as u64 } else { u.ru_maxrss as u64 * 1024 }
}

#[cfg(test)]
mod tests {
    use crate::{hier, tile};

    #[test]
    fn files_equal_memory() {
        // a ring of 12 nodes in both directions, in two zoom-8 tiles and with some coarse nodes
        let n = 12u32;
        let mut turns = Vec::new();
        for v in 0..n {
            turns.push((v, (v + 1) % n, 1.0 + v as f64));
            turns.push(((v + 1) % n, v, 2.0 + v as f64));
        }
        let points = [(43.7, 7.4), (40.0, -80.0)];
        let level: Vec<u8> = (0..n).map(|v| [12, 14, 9, 5][v as usize % 4]).collect();
        let cell: Vec<u64> = (0..n).map(|v| { let (la, lo) = points[v as usize / 6]; tile::key(la, lo, level[v as usize]) }).collect();
        let ids: Vec<i64> = (0..n as i64).map(|v| 1000 + v * 7).collect();
        let h = hier::prepare(&turns, level, cell, 0, true, false);
        let dir = std::env::temp_dir().join(format!("tileroute-test-{}", std::process::id()));
        h.write_files(&ids, &dir).expect("write");
        let store = super::Store::open(&dir, 2).expect("open");
        assert_eq!(store.n_files(), 3);
        for s in 0..n {
            for t in 0..n {
                let mem = h.route(s, t).map(|(c, p)| (c, p.iter().map(|&v| ids[v as usize]).collect::<Vec<i64>>()));
                let (disk, st) = store.route(ids[s as usize], ids[t as usize]).expect("route");
                assert_eq!(disk, mem, "{s} -> {t}");
                assert!(st.tiles >= 1 && st.bytes > 0);
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }
}
