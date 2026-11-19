//! Cells from the road graph (step D1 of `docs/explanation/distributed.md`): an inertial flow partition.
//!
//! The junctions are cut in two again and again, each time where few roads cross. Two cuts make one level, so a cell
//! has 4 children and its key has the same form as a tile key (`crate::tile`): `parent` and `lca_zoom` work unchanged,
//! and the level of a road edge is the `lca_zoom` of the keys of its two junctions, as with tiles.

use rayon::prelude::*;
use std::collections::VecDeque;

/// The cut of one cell in two: for each of 4 lines (west-east, south-north and the two diagonals), the junctions in
/// order along the line; the first quarter is the source side, the last quarter the target side; the minimum cut
/// between them (maximum flow, each road edge capacity 1). The line with the smallest cut wins. Returns the two halves.
fn bisect(nodes: &[u32], xy: &[(f64, f64)], adj: &Adjacency) -> (Vec<u32>, Vec<u32>, usize) {
    // sorted, so that the local number of a junction is a binary search (no array of all junctions for each cell)
    let mut nodes = nodes.to_vec();
    nodes.sort_unstable();
    let nodes = &nodes[..];
    let lines: [fn((f64, f64)) -> f64; 4] = [|p| p.0, |p| p.1, |p| p.0 + p.1, |p| p.0 - p.1];
    let best = lines
        .par_iter()
        .map(|line| {
            let mut order: Vec<u32> = (0..nodes.len() as u32).collect();
            order.sort_unstable_by(|&a, &b| line(xy[nodes[a as usize] as usize]).total_cmp(&line(xy[nodes[b as usize] as usize])));
            let q = (nodes.len() / 4).max(1);
            min_cut(nodes, adj, &order[..q], &order[order.len() - q..])
        })
        .min_by_key(|(side, cut)| (*cut, side.iter().filter(|&&s| s).count().abs_diff(nodes.len() / 2)))
        .expect("4 lines");
    let (side, cut) = best;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for (i, &v) in nodes.iter().enumerate() {
        if side[i] { a.push(v) } else { b.push(v) }
    }
    (a, b, cut)
}

/// The road graph between junctions, with no direction: for each junction, (other junction, number of road edges).
pub struct Adjacency {
    off: Vec<u32>,
    arc: Vec<(u32, u32)>,
}

impl Adjacency {
    /// `edges`: (from junction, to junction) of each road edge; a loop is left out.
    pub fn new(n: usize, edges: &[(u32, u32)]) -> Adjacency {
        let mut pairs: Vec<(u32, u32)> = edges.iter().filter(|e| e.0 != e.1).flat_map(|&(a, b)| [(a, b), (b, a)]).collect();
        pairs.par_sort_unstable();
        let mut off = vec![0u32; n + 1];
        let mut arc: Vec<(u32, u32)> = Vec::new();
        for (i, &(a, b)) in pairs.iter().enumerate() {
            if i > 0 && pairs[i - 1] == (a, b) {
                arc.last_mut().expect("the same pair").1 += 1;
            } else {
                arc.push((b, 1));
                off[a as usize + 1] += 1;
            }
        }
        for i in 1..=n {
            off[i] += off[i - 1];
        }
        Adjacency { off, arc }
    }

    fn of(&self, v: u32) -> &[(u32, u32)] {
        &self.arc[self.off[v as usize] as usize..self.off[v as usize + 1] as usize]
    }
}

/// The minimum cut between the source and the target junctions (local numbers), with Dinic's maximum flow on the
/// junctions of the cell; the source side is all junctions that the rest graph reaches from the source. Returns the
/// side of each junction (true: the source side) and the size of the cut (road edges).
fn min_cut(nodes: &[u32], adj: &Adjacency, src: &[u32], dst: &[u32]) -> (Vec<bool>, usize) {
    // the flow graph: node 0 = all source junctions, node 1 = all target junctions, 2.. = the other junctions
    const S: u32 = 0;
    const T: u32 = 1;
    let k = nodes.len();
    let mut id = vec![u32::MAX; k];
    src.iter().for_each(|&i| id[i as usize] = S);
    dst.iter().for_each(|&i| id[i as usize] = T);
    let mut next = 2u32;
    for x in id.iter_mut().filter(|x| **x == u32::MAX) {
        *x = next;
        next += 1;
    }
    let m = next as usize;
    // arcs as pairs: arc 2j and 2j + 1 are the two directions of one road link (capacity c in each direction)
    let (mut to, mut cap, mut head) = (Vec::new(), Vec::new(), vec![u32::MAX; m]);
    let mut link = Vec::new();
    let mut add = |a: u32, b: u32, c: u32, to: &mut Vec<u32>, cap: &mut Vec<i64>, link: &mut Vec<u32>| {
        for (x, y) in [(a, b), (b, a)] {
            to.push(y);
            cap.push(c as i64);
            link.push(head[x as usize]);
            head[x as usize] = to.len() as u32 - 1;
        }
    };
    for (i, &v) in nodes.iter().enumerate() {
        for &(w, c) in adj.of(v) {
            // each link once (v < w), only inside the cell, and not inside the source or the target side
            if v >= w {
                continue;
            }
            let Ok(j) = nodes.binary_search(&w) else { continue };
            let (a, b) = (id[i], id[j]);
            if a != b {
                add(a, b, c, &mut to, &mut cap, &mut link);
            }
        }
    }
    let mut flow = 0i64;
    let mut dist = vec![u32::MAX; m];
    let mut it = vec![0u32; m];
    loop {
        // the levels of Dinic, from S
        dist.iter_mut().for_each(|d| *d = u32::MAX);
        dist[S as usize] = 0;
        let mut q = VecDeque::from([S]);
        while let Some(u) = q.pop_front() {
            let mut e = head[u as usize];
            while e != u32::MAX {
                let w = to[e as usize];
                if cap[e as usize] > 0 && dist[w as usize] == u32::MAX {
                    dist[w as usize] = dist[u as usize] + 1;
                    q.push_back(w);
                }
                e = link[e as usize];
            }
        }
        if dist[T as usize] == u32::MAX {
            break;
        }
        it.copy_from_slice(&head);
        // the blocking flow, one path at a time, with no recursion
        loop {
            let mut path: Vec<u32> = Vec::new();
            let mut u = S;
            while u != T {
                let mut e = it[u as usize];
                while e != u32::MAX && !(cap[e as usize] > 0 && dist[to[e as usize] as usize] == dist[u as usize] + 1) {
                    e = link[e as usize];
                }
                it[u as usize] = e;
                if e == u32::MAX {
                    // a dead end: no path from here in this phase
                    dist[u as usize] = u32::MAX;
                    match path.pop() {
                        Some(back) => u = to[(back ^ 1) as usize],
                        None => break,
                    }
                    continue;
                }
                path.push(e);
                u = to[e as usize];
            }
            if u != T {
                break;
            }
            let push = path.iter().map(|&e| cap[e as usize]).min().expect("a path");
            for &e in &path {
                cap[e as usize] -= push;
                cap[(e ^ 1) as usize] += push;
            }
            flow += push;
        }
    }
    // the source side: what the rest graph reaches from S (dist after the last BFS)
    let side = (0..k).map(|i| dist[id[i] as usize] != u32::MAX).collect();
    (side, flow as usize)
}

/// The cell key of each junction, at zoom `depth`: cut until a cell has at most `leaf` junctions or the zoom is
/// `depth`. `xy`: the position of each junction (any plane projection). Also returns the size of the cuts by zoom.
pub fn partition(xy: &[(f64, f64)], adj: &Adjacency, leaf: usize, depth: u8) -> (Vec<u64>, Vec<usize>) {
    let n = xy.len();
    let mut key = vec![0u64; n];
    let mut cuts = vec![0usize; depth as usize + 1];
    // the cells of one zoom: (junctions, digits); all cells of one zoom in parallel
    let mut cells: Vec<(Vec<u32>, u64)> = vec![((0..n as u32).collect(), 0)];
    let shift = |z: u8| 6 + 2 * (crate::tile::MAX_ZOOM - z) as u32;
    for z in 0..depth {
        let (done, open): (Vec<_>, Vec<_>) = cells.into_iter().partition(|c| c.0.len() <= leaf);
        for (vs, d) in done {
            vs.iter().for_each(|&v| key[v as usize] = (d << shift(z)) | depth as u64);
        }
        // the cells of one zoom in parallel, and the 4 lines of each cut too
        let split: Vec<(Vec<(Vec<u32>, u64)>, usize)> = open
            .into_par_iter()
            .map(|(vs, d)| {
                let (a, b, c1) = bisect(&vs, xy, adj);
                let ((a0, a1, c2), (b0, b1, c3)) = rayon::join(|| bisect(&a, xy, adj), || bisect(&b, xy, adj));
                let parts = [a0, a1, b0, b1].into_iter().enumerate().filter(|p| !p.1.is_empty()).map(|(q, p)| (p, (d << 2) | q as u64)).collect();
                (parts, c1 + c2 + c3)
            })
            .collect();
        cuts[z as usize] = split.iter().map(|s| s.1).sum();
        cells = split.into_iter().flat_map(|s| s.0).collect();
    }
    for (vs, d) in cells {
        vs.iter().for_each(|&v| key[v as usize] = (d << shift(depth)) | depth as u64);
    }
    (key, cuts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_cut_in_the_middle() {
        // a 20 x 20 grid with one road between neighbours; the best cut of a square grid is a middle line: 20 roads
        let k = 20u32;
        let xy: Vec<(f64, f64)> = (0..k * k).map(|v| ((v % k) as f64, (v / k) as f64)).collect();
        let mut edges = Vec::new();
        for v in 0..k * k {
            if v % k + 1 < k {
                edges.push((v, v + 1));
            }
            if v / k + 1 < k {
                edges.push((v, v + k));
            }
        }
        let adj = Adjacency::new((k * k) as usize, &edges);
        let all: Vec<u32> = (0..k * k).collect();
        let (a, b, cut) = bisect(&all, &xy, &adj);
        assert_eq!(cut, 20);
        assert_eq!(a.len() + b.len(), 400);
        // the keys: nested, and a road inside a small cell has a fine level
        // the keys: all at zoom 8; the roads of each level are exactly the roads of the cuts of that zoom
        let (key, cuts) = partition(&xy, &adj, 4, 8);
        assert!(key.iter().all(|&x| crate::tile::zoom(x) == 8));
        for z in 0..8u8 {
            let at = edges.iter().filter(|&&(a, b)| crate::tile::lca_zoom(key[a as usize], key[b as usize]) == z).count();
            assert_eq!(at, cuts[z as usize], "zoom {z}");
        }
        assert!(cuts[0] <= 20 + 2 * 12, "{}", cuts[0]);
    }
}
