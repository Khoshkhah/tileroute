//! A structure with no costs, then a customization that adds the costs (as a customizable contraction hierarchy).
//!
//! 1. **Order** ([`Cch::new`]): each node gets its own rank, from the cells: the finest level first, in a level by
//!    cell, in a cell by node number. With the partition cells this is a nested-dissection order.
//! 2. **Structure, no costs**: remove the nodes one at a time by rank and join all their higher neighbours (no witness
//!    search). The result depends only on the graph and the order, so it is made once.
//! 3. **Customization** ([`Cch::customize`]): the costs of one metric. For each node x by rank and each pair y, z of
//!    its higher neighbours: cost(y → z) = min(cost(y → z), cost(y → x) + cost(x → z)). Each edge keeps the x of its best
//!    cost (the unpack). Then the arc reduction removes the search arcs that the costs do not need.
//! 4. **Query** ([`Metric::route`]): a bidirectional search up the ranks, as in a contraction hierarchy.
//!
//! The ranks are the node numbers inside this module; `order` gives the input node of each rank.

use crate::hier::{reduce, Fast};
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// The structure: for each rank, its higher neighbours (ranks, sorted). An edge is (lower rank, index in its row).
pub struct Cch {
    pub order: Vec<u32>,
    pub rank: Vec<u32>,
    off: Vec<u32>,
    up: Vec<u32>,
    /// the rank of the level of each rank (for the parallel customization: the levels of the elimination tree)
    depth: Vec<u32>,
}

impl Cch {
    /// The order from `level` and `cell` (finest level first), and the structure of the graph of `arcs` (u, w).
    pub fn new(n: usize, arcs: impl Iterator<Item = (u32, u32)>, level: &[u8], cell: &[u64]) -> Cch {
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.par_sort_unstable_by_key(|&v| (std::cmp::Reverse(level[v as usize]), cell[v as usize], v));
        let mut rank = vec![0u32; n];
        for (r, &v) in order.iter().enumerate() {
            rank[v as usize] = r as u32;
        }
        // the higher neighbours of each rank, then the elimination: the lowest higher neighbour p of r (its parent in
        // the elimination tree) gets the other higher neighbours of r; this gives all edges of the chordal graph
        let mut up: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (u, w) in arcs {
            let (a, b) = (rank[u as usize], rank[w as usize]);
            if a != b {
                up[a.min(b) as usize].push(a.max(b));
            }
        }
        let mut depth = vec![0u32; n];
        for r in 0..n {
            let mut row = std::mem::take(&mut up[r]);
            row.sort_unstable();
            row.dedup();
            if let Some(&p) = row.first() {
                up[p as usize].extend_from_slice(&row[1..]);
                depth[p as usize] = depth[p as usize].max(depth[r] + 1);
            }
            up[r] = row;
        }
        let mut off = vec![0u32];
        let mut flat = Vec::with_capacity(up.iter().map(Vec::len).sum());
        for row in up {
            flat.extend_from_slice(&row);
            off.push(flat.len() as u32);
        }
        Cch { order, rank, off, up: flat, depth }
    }

    pub fn n_edges(&self) -> usize {
        self.up.len()
    }

    fn row(&self, r: u32) -> std::ops::Range<usize> {
        self.off[r as usize] as usize..self.off[r as usize + 1] as usize
    }

    /// The edge of the ranks a < b.
    fn edge(&self, a: u32, b: u32) -> Option<usize> {
        let r = self.row(a);
        self.up[r.clone()].binary_search(&b).ok().map(|i| r.start + i)
    }

    /// The costs of one metric: `arcs` (u, w, cost) of the input graph (the cheapest cost of a pair counts).
    /// `reduce_arcs`: then the arc reduction removes the search arcs that these costs do not need.
    pub fn customize(&self, arcs: &[(u32, u32, f64)], reduce_arcs: bool) -> Metric<'_> {
        let t0 = std::time::Instant::now();
        let m = self.up.len();
        // for each edge (a lower, b higher): up = cost a → b, down = cost b → a; via = the x of the best cost
        let mut up = vec![f64::INFINITY; m];
        let mut down = vec![f64::INFINITY; m];
        for &(u, w, c) in arcs {
            let (a, b) = (self.rank[u as usize], self.rank[w as usize]);
            if a == b {
                continue;
            }
            let e = self.edge(a.min(b), a.max(b)).expect("an input arc is an edge");
            let x = if a < b { &mut up[e] } else { &mut down[e] };
            if c < *x {
                *x = c;
            }
        }
        let mut via_up = vec![u32::MAX; m];
        let mut via_down = vec![u32::MAX; m];
        // ponytail: one thread, by rank; the parallel version runs the ranks of one depth of the elimination tree
        // together, when the time matters
        for x in 0..self.order.len() as u32 {
            let r = self.row(x);
            for i in r.clone() {
                let y = self.up[i];
                // y → x and x → y, the costs of the edge (x, y)
                let (yx, xy) = (down[i], up[i]);
                if yx.is_infinite() && xy.is_infinite() {
                    continue;
                }
                for j in i + 1..r.end {
                    let z = self.up[j];
                    let (zx, xz) = (down[j], up[j]);
                    let e = self.edge(y, z).expect("the graph is chordal");
                    // y < z: up[e] = y → z, down[e] = z → y
                    if yx + xz < up[e] {
                        up[e] = yx + xz;
                        via_up[e] = x;
                    }
                    if zx + xy < down[e] {
                        down[e] = zx + xy;
                        via_down[e] = x;
                    }
                }
            }
        }
        let n = self.order.len();
        // the search graphs: forward along up costs, backward along down costs; then the arc reduction
        let search = |cost: &[f64]| -> Fast {
            let mut off = vec![0u32];
            let mut arc = Vec::new();
            for a in 0..n as u32 {
                for e in self.row(a) {
                    if cost[e].is_finite() {
                        arc.push((cost[e], self.up[e]));
                    }
                }
                off.push(arc.len() as u32);
            }
            Fast { off, arc }
        };
        let (fwd, bwd) = (search(&up), search(&down));
        let before = fwd.arc.len() + bwd.arc.len();
        let t_tri = t0.elapsed().as_secs_f64();
        let (fwd, bwd) = if reduce_arcs { (reduce(fwd), reduce(bwd)) } else { (fwd, bwd) };
        let times = (t_tri, t0.elapsed().as_secs_f64() - t_tri);
        Metric { cch: self, up, down, via_up, via_down, fwd, bwd, before, times }
    }

    pub fn max_depth(&self) -> u32 {
        self.depth.iter().copied().max().unwrap_or(0)
    }
}

/// The costs of one metric on a [`Cch`].
pub struct Metric<'a> {
    cch: &'a Cch,
    up: Vec<f64>,
    down: Vec<f64>,
    via_up: Vec<u32>,
    via_down: Vec<u32>,
    fwd: Fast,
    bwd: Fast,
    /// the search arcs before the arc reduction
    pub before: usize,
    /// seconds: the triangles, the arc reduction
    pub times: (f64, f64),
}

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

/// What one query did.
#[derive(Default, Clone, Copy)]
pub struct Stats {
    pub settled: usize,
    pub relaxed: usize,
}

impl Metric<'_> {
    /// The search arcs after the reduction (forward + backward).
    pub fn n_search_arcs(&self) -> usize {
        self.fwd.arc.len() + self.bwd.arc.len()
    }

    /// The shortest path from input node `s` to `t`: its cost and its input nodes.
    pub fn route(&self, s: u32, t: u32) -> (Option<(f64, Vec<u32>)>, Stats) {
        let c = self.cch;
        let mut st = Stats::default();
        if s == t {
            return (Some((0.0, vec![s])), st);
        }
        let (s, t) = (c.rank[s as usize], c.rank[t as usize]);
        let mut label: [HashMap<u32, (f64, u32)>; 2] = Default::default();
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
            st.settled += 1;
            if let Some(&(e, _)) = label[1 - dir].get(&v) {
                if d + e < best {
                    best = d + e;
                    meet = v;
                }
            }
            let g = if dir == 0 { &self.fwd } else { &self.bwd };
            for &(cost, w) in g.of(v) {
                st.relaxed += 1;
                let nd = d + cost;
                if label[dir].get(&w).is_none_or(|&(e, _)| nd < e) {
                    label[dir].insert(w, (nd, v));
                    heap[dir].push(Key(nd, w));
                }
            }
        }
        if meet == u32::MAX {
            return (None, st);
        }
        let chain = |dir: usize| {
            let mut v = vec![meet];
            while let Some(&(_, p)) = label[dir].get(v.last().expect("not empty")) {
                if p == u32::MAX {
                    break;
                }
                v.push(p);
            }
            v
        };
        let mut ranks = chain(0);
        ranks.reverse();
        ranks.extend(chain(1).into_iter().skip(1));
        let mut path = vec![s];
        for w in ranks.windows(2) {
            self.unpack(w[0], w[1], &mut path);
        }
        (Some((best, path.into_iter().map(|r| c.order[r as usize]).collect())), st)
    }

    /// Add the ranks of the arc a → b after a (a is already in `path`).
    fn unpack(&self, a: u32, b: u32, path: &mut Vec<u32>) {
        let (lo, hi) = (a.min(b), a.max(b));
        let e = self.cch.edge(lo, hi).expect("an arc of the path is an edge");
        let x = if a < b { self.via_up[e] } else { self.via_down[e] };
        if x == u32::MAX {
            path.push(b);
        } else {
            self.unpack(a, x, path);
            self.unpack(x, b, path);
        }
    }

    /// The cost of the arc a → b (ranks), for checks.
    pub fn cost(&self, a: u32, b: u32) -> f64 {
        let e = self.cch.edge(a.min(b), a.max(b)).expect("an edge");
        if a < b { self.up[e] } else { self.down[e] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_equal_to_dijkstra() {
        // a ring of 12 nodes in both directions with other costs each way, and two chords; any order is exact
        let n = 12u32;
        let mut arcs = Vec::new();
        for v in 0..n {
            arcs.push((v, (v + 1) % n, 1.0 + v as f64));
            arcs.push(((v + 1) % n, v, 2.0 + (v * 7 % 5) as f64));
        }
        arcs.push((0, 6, 4.0));
        arcs.push((9, 3, 3.0));
        let level: Vec<u8> = (0..n).map(|v| [3, 5, 2, 5][v as usize % 4]).collect();
        let cell: Vec<u64> = (0..n as u64).map(|v| v / 6).collect();
        let cch = Cch::new(n as usize, arcs.iter().map(|a| (a.0, a.1)), &level, &cell);
        let m = cch.customize(&arcs, true);
        // Dijkstra on the input
        let dist = |s: u32| {
            let mut d = vec![f64::INFINITY; n as usize];
            d[s as usize] = 0.0;
            for _ in 0..n {
                for &(u, w, c) in &arcs {
                    if d[u as usize] + c < d[w as usize] {
                        d[w as usize] = d[u as usize] + c;
                    }
                }
            }
            d
        };
        for s in 0..n {
            let d = dist(s);
            for t in 0..n {
                let (r, _) = m.route(s, t);
                let (c, p) = r.expect("a route");
                assert_eq!(c, d[t as usize], "{s} -> {t}");
                assert_eq!((p[0], *p.last().unwrap()), (s, t));
                let sum: f64 = p.windows(2).map(|w| arcs.iter().filter(|a| a.0 == w[0] && a.1 == w[1]).map(|a| a.2).fold(f64::INFINITY, f64::min)).sum();
                assert_eq!(sum, c, "the path of {s} -> {t}");
            }
        }
    }
}
