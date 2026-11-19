//! The triangle version: a customizable contraction hierarchy whose order comes from the tiles.
//!
//! **Order.** The nodes are removed one at a time: the finest level first, then the coarser levels (inside one level, by
//! cell, then by node number). This is the same order as the sets of [`crate::hier`], but node by node.
//!
//! **Structure** (no costs). When a node `x` is removed, each two of its neighbours that are not removed yet get an edge
//! (a shortcut place), if they have none. An edge `{p, q}` (p removed before q) has two costs: `p -> q` and `q -> p`.
//! The structure depends only on the graph and the order, never on the costs.
//!
//! **Costs** (customization). The cost of `p -> q` is the smaller of its own arc (infinity if none) and, for each node
//! `x` removed before p and q with edges to both (a lower triangle), `x`'s two sides: `cost(p -> x) + cost(x -> q)`.
//! A full customization does all edges in the order of removal. An update after a change of some arcs computes again only
//! the edges whose cost can change, and stops where a cost does not change.
//!
//! **Query.** The search of a contraction hierarchy: forward from `s` and backward from `t`, each only to nodes removed
//! later.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

/// A heap entry, the smallest key first.
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

#[derive(Clone)]
pub struct Cch {
    /// the place of each node in the order of removal
    rank: Vec<u32>,
    /// the edges to nodes removed later: node v has edges `up_off[v]..up_off[v + 1]`, to `up_to`, sorted by node
    up_off: Vec<u32>,
    up_to: Vec<u32>,
    /// the edges from nodes removed before: (other node, edge), sorted by node
    down_off: Vec<u32>,
    down: Vec<(u32, u32)>,
    /// the cost of the arc of the input graph, for each edge `{p, q}`: `p -> q` (fw) and `q -> p` (bw); infinity: no arc
    base_fw: Vec<f64>,
    base_bw: Vec<f64>,
    /// the costs after customization
    fw: Vec<f64>,
    bw: Vec<f64>,
}

impl Cch {
    /// The structure of the graph `arcs` (`(from, to, cost)`, each pair once) with the order from `level` and `cell`,
    /// then a full customization.
    pub fn new(n: usize, arcs: &[(u32, u32, f64)], level: &[u8], cell: &[u64]) -> Cch {
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by_key(|&v| (std::cmp::Reverse(level[v as usize]), cell[v as usize], v));
        let mut rank = vec![0u32; n];
        for (i, &v) in order.iter().enumerate() {
            rank[v as usize] = i as u32;
        }
        // the elimination game: the neighbours of each node that are removed later
        let mut up: Vec<Vec<u32>> = vec![Vec::new(); n];
        for &(a, b, _) in arcs {
            if a == b {
                continue;
            }
            if rank[a as usize] < rank[b as usize] {
                up[a as usize].push(b);
            } else {
                up[b as usize].push(a);
            }
        }
        for &x in &order {
            let mut u = std::mem::take(&mut up[x as usize]);
            u.sort_unstable();
            u.dedup();
            for i in 0..u.len() {
                for j in i + 1..u.len() {
                    let (p, q) = if rank[u[i] as usize] < rank[u[j] as usize] { (u[i], u[j]) } else { (u[j], u[i]) };
                    up[p as usize].push(q);
                }
            }
            up[x as usize] = u;
        }
        let mut up_off = vec![0u32; n + 1];
        for v in 0..n {
            up_off[v + 1] = up_off[v] + up[v].len() as u32;
        }
        let up_to: Vec<u32> = up.into_iter().flatten().collect();
        let m = up_to.len();
        // the edges from below, by node
        let mut down_off = vec![0u32; n + 1];
        for &w in &up_to {
            down_off[w as usize + 1] += 1;
        }
        for v in 0..n {
            down_off[v + 1] += down_off[v];
        }
        let mut fill = down_off.clone();
        let mut down = vec![(0u32, 0u32); m];
        for x in 0..n {
            for e in up_off[x] as usize..up_off[x + 1] as usize {
                let w = up_to[e] as usize;
                down[fill[w] as usize] = (x as u32, e as u32);
                fill[w] += 1;
            }
        }
        // x goes from 0 to n, so each `down` list is already sorted by node
        let mut c = Cch {
            rank,
            up_off,
            up_to,
            down_off,
            down,
            base_fw: vec![f64::INFINITY; m],
            base_bw: vec![f64::INFINITY; m],
            fw: Vec::new(),
            bw: Vec::new(),
        };
        for &(a, b, w) in arcs {
            if a != b {
                c.set_base(a, b, w);
            }
        }
        c.customize();
        c
    }

    /// The edge `{a, b}`, and whether `a` is removed before `b`.
    fn edge(&self, a: u32, b: u32) -> (usize, bool) {
        let (p, q, a_low) = if self.rank[a as usize] < self.rank[b as usize] { (a, b, true) } else { (b, a, false) };
        let r = self.up_off[p as usize] as usize..self.up_off[p as usize + 1] as usize;
        let k = self.up_to[r.clone()].binary_search(&q).expect("an arc is an edge");
        (r.start + k, a_low)
    }

    pub fn set_base(&mut self, a: u32, b: u32, w: f64) {
        let (e, a_low) = self.edge(a, b);
        if a_low {
            self.base_fw[e] = w;
        } else {
            self.base_bw[e] = w;
        }
    }

    /// The number of edges, and of edges that are not an arc of the input graph (shortcuts only).
    pub fn size(&self) -> (usize, usize) {
        let only = (0..self.up_to.len()).filter(|&e| self.base_fw[e].is_infinite() && self.base_bw[e].is_infinite()).count();
        (self.up_to.len(), only)
    }

    /// The full customization: each node in the order of removal gives its triangles to the edges between its upper
    /// neighbours.
    pub fn customize(&mut self) {
        self.fw = self.base_fw.clone();
        self.bw = self.base_bw.clone();
        let n = self.rank.len();
        let mut order: Vec<u32> = (0..n as u32).collect();
        order.sort_unstable_by_key(|&v| self.rank[v as usize]);
        for x in order {
            let r = self.up_off[x as usize] as usize..self.up_off[x as usize + 1] as usize;
            for i in r.clone() {
                for j in r.clone() {
                    if i == j {
                        continue;
                    }
                    let (u, w) = (self.up_to[i], self.up_to[j]);
                    // only the pairs with u removed before w; edge e = {u, w}
                    if self.rank[u as usize] > self.rank[w as usize] {
                        continue;
                    }
                    let (e, _) = self.edge(u, w);
                    // u -> x -> w and w -> x -> u
                    let f = self.bw[i] + self.fw[j];
                    let b = self.bw[j] + self.fw[i];
                    if f < self.fw[e] {
                        self.fw[e] = f;
                    }
                    if b < self.bw[e] {
                        self.bw[e] = b;
                    }
                }
            }
        }
    }

    /// The cost of edge `e = {p, q}` from its arc and its lower triangles: (p -> q, q -> p).
    fn compute(&self, e: usize, p: u32, q: u32) -> (f64, f64) {
        let (mut f, mut b) = (self.base_fw[e], self.base_bw[e]);
        let lp = &self.down[self.down_off[p as usize] as usize..self.down_off[p as usize + 1] as usize];
        let lq = &self.down[self.down_off[q as usize] as usize..self.down_off[q as usize + 1] as usize];
        let (mut i, mut j) = (0, 0);
        while i < lp.len() && j < lq.len() {
            match lp[i].0.cmp(&lq[j].0) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    let (exp, exq) = (lp[i].1 as usize, lq[j].1 as usize);
                    // edge {x, p}: fw = x -> p, bw = p -> x
                    f = f.min(self.bw[exp] + self.fw[exq]);
                    b = b.min(self.bw[exq] + self.fw[exp]);
                    i += 1;
                    j += 1;
                }
            }
        }
        (f, b)
    }

    /// Change the costs of some arcs of the input graph (`(from, to, new cost)`), then compute again only the edges whose
    /// cost can change. Returns the number of edges computed again from all their triangles.
    ///
    /// An edge is done in the order of its lower node, so its triangles are final before it. When an edge changes, each
    /// edge above it (with this edge as a side of a triangle) gets one test: it is computed again from all its triangles
    /// only if the new triangle is cheaper than its cost, or if the old triangle gave its cost and the new one is more
    /// expensive. All other edges keep their costs with no work.
    pub fn update(&mut self, changes: &[(u32, u32, f64)]) -> usize {
        let mut node_of_rank = vec![0u32; self.rank.len()];
        for (v, &r) in self.rank.iter().enumerate() {
            node_of_rank[r as usize] = v as u32;
        }
        // the queue: (rank of the lower node, edge); `old`: the costs of an edge before this update; `full`: compute it
        // again from all its triangles
        let mut queue: BinaryHeap<std::cmp::Reverse<(u32, u32)>> = BinaryHeap::new();
        let mut old: HashMap<u32, (f64, f64)> = HashMap::new();
        let mut full: HashMap<u32, bool> = HashMap::new();
        let low = |c: &Cch, a: u32, b: u32| if c.rank[a as usize] < c.rank[b as usize] { a } else { b };
        for &(a, b, w) in changes {
            self.set_base(a, b, w);
            let (e, _) = self.edge(a, b);
            old.entry(e as u32).or_insert((self.fw[e], self.bw[e]));
            if full.insert(e as u32, true).is_none() {
                queue.push(std::cmp::Reverse((self.rank[low(self, a, b) as usize], e as u32)));
            } else {
                full.insert(e as u32, true);
            }
        }
        let mut done = 0;
        while let Some(std::cmp::Reverse((rp, e))) = queue.pop() {
            let again = full.remove(&e).unwrap_or(false);
            let e = e as usize;
            let p = node_of_rank[rp as usize];
            let q = self.up_to[e];
            if again {
                let (f, b) = self.compute(e, p, q);
                self.fw[e] = f;
                self.bw[e] = b;
                done += 1;
            }
            let (of, ob) = old[&(e as u32)];
            let (nf, nb) = (self.fw[e], self.bw[e]);
            if (nf, nb) == (of, ob) {
                continue;
            }
            // e = {p, q} (fw: p -> q) is a side of the triangles with bottom p: the edges {q, y}, with side k = {p, y}
            for k in self.up_off[p as usize] as usize..self.up_off[p as usize + 1] as usize {
                let y = self.up_to[k];
                if y == q {
                    continue;
                }
                let (e2, q_low) = self.edge(q, y);
                let (kf, kb) = old.get(&(k as u32)).copied().unwrap_or((self.fw[k], self.bw[k]));
                let (ckf, ckb) = (self.fw[k], self.bw[k]);
                // q -> p -> y and y -> p -> q, before and now
                let (o_qy, o_yq) = (ob + kf, kb + of);
                let (n_qy, n_yq) = (nb + ckf, ckb + nf);
                // the two costs of e2 as q -> y and y -> q
                let (c_qy, c_yq) = if q_low { (self.fw[e2], self.bw[e2]) } else { (self.bw[e2], self.fw[e2]) };
                // compute e2 again (when its turn comes, all its sides are final) only if a triangle became cheaper than
                // its cost, or if the triangle that gave its cost became more expensive
                let touch = |o: f64, nw: f64, c: f64| nw < c || (o == c && nw > o);
                if !touch(o_qy, n_qy, c_qy) && !touch(o_yq, n_yq, c_yq) {
                    continue;
                }
                let e2u = e2 as u32;
                old.entry(e2u).or_insert((self.fw[e2], self.bw[e2]));
                if full.insert(e2u, true).is_none() {
                    queue.push(std::cmp::Reverse((self.rank[low(self, q, y) as usize], e2u)));
                }
            }
        }
        done
    }

    /// The cost of the arc `a -> b` (an edge of the structure) after customization.
    fn cost(&self, a: u32, b: u32) -> f64 {
        let (e, a_low) = self.edge(a, b);
        if a_low { self.fw[e] } else { self.bw[e] }
    }

    /// Do the costs equal those of another customization (the same structure)?
    pub fn same_costs(&self, o: &Cch) -> bool {
        self.fw == o.fw && self.bw == o.bw
    }

    /// The shortest path from `s` to `t`: its cost and its nodes. None: no path.
    pub fn route(&self, s: u32, t: u32) -> Option<(f64, Vec<u32>)> {
        if s == t {
            return Some((0.0, vec![s]));
        }
        let mut label: [HashMap<u32, (f64, u32)>; 2] = [HashMap::new(), HashMap::new()];
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
            if let Some(&(o, _)) = label[1 - dir].get(&v) {
                if d + o < best {
                    best = d + o;
                    meet = v;
                }
            }
            for e in self.up_off[v as usize] as usize..self.up_off[v as usize + 1] as usize {
                let w = self.up_to[e];
                // forward: v -> w (fw); backward: w -> v (bw)
                let c = if dir == 0 { self.fw[e] } else { self.bw[e] };
                let nd = d + c;
                if nd.is_finite() && label[dir].get(&w).is_none_or(|&(o, _)| nd < o) {
                    label[dir].insert(w, (nd, v));
                    heap[dir].push(Key(nd, w));
                }
            }
        }
        if meet == u32::MAX {
            return None;
        }
        let mut top_path = vec![meet];
        while let Some(&(_, p)) = label[0].get(top_path.last().expect("not empty")) {
            if p == u32::MAX {
                break;
            }
            top_path.push(p);
        }
        top_path.reverse();
        let mut v = meet;
        while let Some(&(_, p)) = label[1].get(&v) {
            if p == u32::MAX {
                break;
            }
            top_path.push(p);
            v = p;
        }
        let mut path = vec![s];
        for w in top_path.windows(2) {
            self.unpack(w[0], w[1], &mut path);
        }
        Some((best, path))
    }

    /// Add the nodes of the arc `a -> b` after `a`: the arc itself if its own cost is the cost, else the lower triangle
    /// that gives the cost.
    fn unpack(&self, a: u32, b: u32, path: &mut Vec<u32>) {
        let (e, a_low) = self.edge(a, b);
        let (c, base) = if a_low { (self.fw[e], self.base_fw[e]) } else { (self.bw[e], self.base_bw[e]) };
        if base == c {
            path.push(b);
            return;
        }
        let (p, q) = if a_low { (a, b) } else { (b, a) };
        let lp = &self.down[self.down_off[p as usize] as usize..self.down_off[p as usize + 1] as usize];
        let lq = &self.down[self.down_off[q as usize] as usize..self.down_off[q as usize + 1] as usize];
        let (mut i, mut j) = (0, 0);
        while i < lp.len() && j < lq.len() {
            match lp[i].0.cmp(&lq[j].0) {
                Ordering::Less => i += 1,
                Ordering::Greater => j += 1,
                Ordering::Equal => {
                    let x = lp[i].0;
                    if self.cost(a, x) + self.cost(x, b) == c {
                        self.unpack(a, x, path);
                        self.unpack(x, b, path);
                        return;
                    }
                    i += 1;
                    j += 1;
                }
            }
        }
        unreachable!("the cost of an edge comes from its arc or a lower triangle");
    }
}
