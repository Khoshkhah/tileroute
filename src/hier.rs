//! The hierarchy of shortcuts by tile, and the query on it.
//!
//! A graph node is a directed road piece (a duckOSM edge). Its **level** is the zoom of the lowest common ancestor of the
//! tiles of its two end points (`lca_zoom`, at most the finest level), and its **cell** is that ancestor tile. An edge
//! with a high level is short and lies inside a small tile; an edge with a low level crosses the border of large tiles.
//!
//! The preparation goes from the finest level to zoom 1. At zoom `z`, for each tile `T`, the set `X` of the nodes of level
//! `z` with cell `T` is removed. For each node `u` with a turn into `X` and each node `w` with a turn out of `X`, a
//! shortcut `u -> w` gets the cost of the shortest path from `u` to `w` with all its inner nodes in `X`. Two nodes of the
//! same level and different cells never touch (they have no common point), so the tiles of one zoom are independent and
//! run in parallel.
//!
//! This is a contraction hierarchy whose order is given by the tiles: a node of a lower zoom is higher in the hierarchy.
//! The query is the bidirectional search of a contraction hierarchy: forward only to nodes of the same or a lower level,
//! backward the same way.

use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};

/// The `zoom` of an arc that is a turn of the input graph, not a shortcut.
const BASE: u8 = u8::MAX;

/// An arc: its cost, and for a shortcut the zoom and the tile of the set that it jumps over.
#[derive(Clone, Copy, Debug)]
struct Arc {
    cost: f64,
    zoom: u8,
    tile: u64,
}

/// The arcs of each node in flat arrays, sorted by the other node.
struct Csr {
    off: Vec<u32>,
    other: Vec<u32>,
    arc: Vec<Arc>,
}

impl Csr {
    fn new(n: usize, mut rows: Vec<(u32, u32, Arc)>) -> Csr {
        rows.sort_unstable_by_key(|r| (r.0, r.1));
        let mut off = vec![0u32; n + 1];
        for r in &rows {
            off[r.0 as usize + 1] += 1;
        }
        for i in 0..n {
            off[i + 1] += off[i];
        }
        Csr { off, other: rows.iter().map(|r| r.1).collect(), arc: rows.iter().map(|r| r.2).collect() }
    }

    fn of(&self, v: u32) -> impl Iterator<Item = (u32, Arc)> + '_ {
        (self.off[v as usize] as usize..self.off[v as usize + 1] as usize).map(move |k| (self.other[k], self.arc[k]))
    }

    fn find(&self, u: u32, w: u32) -> Option<Arc> {
        let r = self.off[u as usize] as usize..self.off[u as usize + 1] as usize;
        self.other[r.clone()].binary_search(&w).ok().map(|k| self.arc[r.start + k])
    }
}

/// A heap entry, the smallest distance first; a tie takes the smaller node.
#[derive(PartialEq)]
struct Key(f64, u32);
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

pub struct Hierarchy {
    level: Vec<u8>,
    cell: Vec<u64>,
    /// all arcs (turns and shortcuts), by the node they leave
    all: Csr,
    /// the forward search: arcs `u -> w` with `level[w] <= level[u]`, by `u`
    up: Csr,
    /// the backward search: arcs `u -> w` with `level[u] <= level[w]`, by `w` (`other` is `u`)
    down: Csr,
    /// the number of shortcut pairs made at each zoom
    pub shortcuts: Vec<usize>,
    /// for each zoom: the number of sets, and the sum and the largest of the times of their steps (seconds, one thread)
    pub steps: Vec<(usize, f64, f64)>,
    /// every shortcut pair `(u, w, zoom, tile)` that a step made, also when an arc `u -> w` was cheaper: the pairs depend
    /// only on the network, so a rebuild with new costs uses the same list
    pub pairs: Vec<(u32, u32, u8, u64)>,
}

/// What one search found: the labels (distance, parent) of each direction, the best meeting node, and the tiles (cells)
/// of the nodes that it settled.
struct Search {
    label: [HashMap<u32, (f64, u32)>; 2],
    best: f64,
    meet: u32,
    tiles: std::collections::HashSet<u64>,
}

/// Make the hierarchy. `turns`: `(from, to, cost)`, each pair once; `level` and `cell` of each node.
pub fn prepare(turns: &[(u32, u32, f64)], level: Vec<u8>, cell: Vec<u64>) -> Hierarchy {
    let n = level.len();
    // all arcs that ever were (never removed), and the nodes with an arc into each node
    let mut arcs: Vec<HashMap<u32, Arc>> = vec![HashMap::new(); n];
    let mut inn: Vec<Vec<u32>> = vec![Vec::new(); n];
    for &(u, w, c) in turns {
        arcs[u as usize].insert(w, Arc { cost: c, zoom: BASE, tile: 0 });
        inn[w as usize].push(u);
    }
    let mut alive = vec![true; n];
    let top = level.iter().copied().max().unwrap_or(0);
    let mut shortcuts = vec![0usize; top as usize + 1];
    let mut steps = vec![(0usize, 0.0f64, 0.0f64); top as usize + 1];
    let mut pairs = Vec::new();
    for z in (1..=top).rev() {
        let mut sets: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
        for v in 0..n {
            if level[v] == z {
                sets.entry(cell[v]).or_default().push(v as u32);
            }
        }
        let made: Vec<(u64, Vec<(u32, u32, f64)>, f64)> = sets
            .par_iter()
            .map(|(&t, xs)| {
                let t0 = std::time::Instant::now();
                let rows = contract(&arcs, &inn, &alive, &level, &cell, z, t, xs);
                (t, rows, t0.elapsed().as_secs_f64())
            })
            .collect();
        for (t, rows, secs) in made {
            let st = &mut steps[z as usize];
            st.0 += 1;
            st.1 += secs;
            st.2 = st.2.max(secs);
            for (u, w, c) in rows {
                pairs.push((u, w, z, t));
                shortcuts[z as usize] += 1;
                let new = Arc { cost: c, zoom: z, tile: t };
                match arcs[u as usize].get_mut(&w) {
                    Some(a) if c < a.cost => *a = new,
                    Some(_) => {}
                    None => {
                        arcs[u as usize].insert(w, new);
                        inn[w as usize].push(u);
                    }
                }
            }
        }
        for xs in sets.values() {
            for &x in xs {
                alive[x as usize] = false;
            }
        }
    }
    let mut all = Vec::new();
    let mut up = Vec::new();
    let mut down = Vec::new();
    for (u, m) in arcs.iter().enumerate() {
        for (&w, &a) in m {
            let u = u as u32;
            all.push((u, w, a));
            if level[w as usize] <= level[u as usize] {
                up.push((u, w, a));
            }
            if level[u as usize] <= level[w as usize] {
                down.push((w, u, a));
            }
        }
    }
    Hierarchy { all: Csr::new(n, all), up: Csr::new(n, up), down: Csr::new(n, down), level, cell, shortcuts, steps, pairs }
}

/// The shortcuts over the set `xs` (level `z`, cell `t`): for each node `u` with a turn into the set, a Dijkstra in the
/// set gives the cost to each node `w` out of the set.
#[allow(clippy::too_many_arguments)]
fn contract(
    arcs: &[HashMap<u32, Arc>],
    inn: &[Vec<u32>],
    alive: &[bool],
    level: &[u8],
    cell: &[u64],
    z: u8,
    t: u64,
    xs: &[u32],
) -> Vec<(u32, u32, f64)> {
    let in_set = |v: u32| level[v as usize] == z && cell[v as usize] == t;
    let sources: BTreeSet<u32> =
        xs.iter().flat_map(|&x| inn[x as usize].iter().copied()).filter(|&u| alive[u as usize] && !in_set(u)).collect();
    let mut out = Vec::new();
    for u in sources {
        let mut dist: HashMap<u32, f64> = HashMap::new();
        let mut heap = BinaryHeap::new();
        for (&x, a) in &arcs[u as usize] {
            if in_set(x) && dist.get(&x).is_none_or(|&d| a.cost < d) {
                dist.insert(x, a.cost);
                heap.push(Key(a.cost, x));
            }
        }
        let mut to: BTreeMap<u32, f64> = BTreeMap::new();
        while let Some(Key(d, x)) = heap.pop() {
            if d > dist[&x] {
                continue;
            }
            for (&y, a) in &arcs[x as usize] {
                if !alive[y as usize] {
                    continue;
                }
                let nd = d + a.cost;
                if in_set(y) {
                    if dist.get(&y).is_none_or(|&e| nd < e) {
                        dist.insert(y, nd);
                        heap.push(Key(nd, y));
                    }
                } else if y != u && to.get(&y).is_none_or(|&e| nd < e) {
                    to.insert(y, nd);
                }
            }
        }
        out.extend(to.into_iter().map(|(w, c)| (u, w, c)));
    }
    out
}

impl Hierarchy {
    /// The bidirectional search of a contraction hierarchy.
    fn search(&self, s: u32, t: u32) -> Search {
        let mut r = Search {
            label: [HashMap::new(), HashMap::new()],
            best: f64::INFINITY,
            meet: u32::MAX,
            tiles: Default::default(),
        };
        let mut heap: [BinaryHeap<Key>; 2] = [BinaryHeap::new(), BinaryHeap::new()];
        r.label[0].insert(s, (0.0, u32::MAX));
        r.label[1].insert(t, (0.0, u32::MAX));
        heap[0].push(Key(0.0, s));
        heap[1].push(Key(0.0, t));
        loop {
            let top = |h: &BinaryHeap<Key>| h.peek().map_or(f64::INFINITY, |k| k.0);
            let (f, b) = (top(&heap[0]), top(&heap[1]));
            if f.min(b) >= r.best || (f.is_infinite() && b.is_infinite()) {
                break;
            }
            let dir = if f <= b { 0 } else { 1 };
            let Key(d, v) = heap[dir].pop().expect("not empty");
            if d > r.label[dir][&v].0 {
                continue;
            }
            r.tiles.insert(self.cell[v as usize]);
            if let Some(&(e, _)) = r.label[1 - dir].get(&v) {
                if d + e < r.best {
                    r.best = d + e;
                    r.meet = v;
                }
            }
            let g = if dir == 0 { &self.up } else { &self.down };
            for (w, a) in g.of(v) {
                let nd = d + a.cost;
                if r.label[dir].get(&w).is_none_or(|&(e, _)| nd < e) {
                    r.label[dir].insert(w, (nd, v));
                    heap[dir].push(Key(nd, w));
                }
            }
        }
        r
    }

    /// Forward (`dir` 0): the nodes from `s` to `v`, appended to `path`, which ends with `s`. Backward (`dir` 1): the nodes
    /// from `v` to `t`, appended to `path`, which ends with `v`. Each arc is unpacked.
    fn chain(&self, r: &Search, dir: usize, v: u32, path: &mut Vec<u32>) {
        let mut c = vec![v];
        while let Some(&(_, p)) = r.label[dir].get(c.last().expect("not empty")) {
            if p == u32::MAX {
                break;
            }
            c.push(p);
        }
        if dir == 0 {
            c.reverse();
        }
        for w in c.windows(2) {
            self.unpack(w[0], w[1], path);
        }
    }

    /// The shortest path from node `s` to node `t`: its cost (the sum of the arc costs) and its nodes. None: no path.
    pub fn route(&self, s: u32, t: u32) -> Option<(f64, Vec<u32>)> {
        self.route_stats(s, t).0
    }

    /// [`route`](Self::route), and the number of tiles (cells) of the nodes that the search settled.
    pub fn route_stats(&self, s: u32, t: u32) -> (Option<(f64, Vec<u32>)>, usize) {
        if s == t {
            return (Some((0.0, vec![s])), 1);
        }
        let r = self.search(s, t);
        if r.meet == u32::MAX {
            return (None, r.tiles.len());
        }
        let mut path = vec![s];
        self.chain(&r, 0, r.meet, &mut path);
        self.chain(&r, 1, r.meet, &mut path);
        (Some((r.best, path)), r.tiles.len())
    }

    /// Add the nodes of the arc `a -> b` after `a` (which is already in `path`).
    fn unpack(&self, a: u32, b: u32, path: &mut Vec<u32>) {
        let arc = self.all.find(a, b).expect("an arc of the path is in the graph");
        if arc.zoom == BASE {
            path.push(b);
            return;
        }
        // the shortest path a -> b with its inner nodes in the set (zoom, tile): the same Dijkstra as in `contract`
        let in_set = |v: u32| self.level[v as usize] == arc.zoom && self.cell[v as usize] == arc.tile;
        let mut label: HashMap<u32, (f64, u32)> = HashMap::new();
        let mut heap = BinaryHeap::new();
        for (x, e) in self.all.of(a) {
            if in_set(x) && label.get(&x).is_none_or(|&(d, _)| e.cost < d) {
                label.insert(x, (e.cost, a));
                heap.push(Key(e.cost, x));
            }
        }
        let mut end: Option<(f64, u32)> = None;
        while let Some(Key(d, x)) = heap.pop() {
            if x == b {
                break;
            }
            if d > label[&x].0 {
                continue;
            }
            for (y, e) in self.all.of(x) {
                let nd = d + e.cost;
                if in_set(y) {
                    if label.get(&y).is_none_or(|&(f, _)| nd < f) {
                        label.insert(y, (nd, x));
                        heap.push(Key(nd, y));
                    }
                } else if y == b && end.is_none_or(|(f, _)| nd < f) {
                    end = Some((nd, x));
                    heap.push(Key(nd, b));
                }
            }
        }
        let (cost, mut x) = end.expect("a shortcut has its path in its set");
        debug_assert_eq!(cost, arc.cost);
        let mut inner = vec![];
        while x != a {
            inner.push(x);
            x = label[&x].1;
        }
        inner.reverse();
        let mut prev = a;
        for x in inner.into_iter().chain(std::iter::once(b)) {
            self.unpack(prev, x, path);
            prev = x;
        }
    }

    pub fn n_arcs(&self) -> usize {
        self.all.other.len()
    }
}
