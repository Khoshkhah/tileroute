# Comparison with h3-routing-platform

tileroute takes its main idea from h3-routing-platform: shortcuts grouped by spatial cells, and a search that goes up
from the start and from the target. But it makes a **different set of shortcuts**, and its query uses them in a
different way. This page puts the two methods side by side.

## The two methods

| | h3-routing-platform | tileroute |
|---|---|---|
| Cells | H3 cells, resolution 15 to 0 | XYZ tiles, zoom 20 to 1 |
| Cell of a road edge | the common ancestor of the cells of its two ends (`lca_res`) | the same with tiles (`lca_zoom`) |
| Where a shortcut goes | into the cells of its `inner_cell` (junction) and `outer_cell` (extent), at each resolution between `lca_res` and the cell resolution | not stored by cell: each step removes one set X |
| Which pairs get a shortcut | all pairs of nodes in a cell that have a path in the cell (shortest paths between all pairs) | only pairs `u → w` of border road edges around the set X, with a path through X |
| Passes | up pass (15 → 0), then down pass (0 → 15) | one pass (20 → 1) |
| Shortest-path step | SQL self-joins (Bellman-Ford style) for small cells, scipy for large cells | one Dijkstra in Rust from each `u`, only in X |
| Parallel | 10 worker processes, by cell at resolution 7 | all threads, by tile at each zoom |
| Query | forward: only "up" shortcuts (`inside = 1`); backward: "down" and "lateral" shortcuts; pruned below the LCA resolution | forward and backward: arcs to the same or a coarser level |

## Why the shortcut counts are different

Take a cell with 100 inner nodes, 6 nodes with an arc into it and 6 nodes with an arc out of it:

| | h3-routing-platform | tileroute |
|---|---|---|
| Shortcuts for this cell | up to 100 × 100 pairs, at each resolution and in both passes | at most 6 × 6 = 36, one time |
| How a route leaves the start cell | one shortcut from the inner node to the border | the original arcs inside the cell, to the border |

Both sets of shortcuts give correct routes. The extra shortcuts of h3-routing-platform let its search leave a cell in
one step; tileroute's search walks more steps near the start and the target, but it stores much fewer shortcuts.

So **the number of shortcuts is a result of each method, not a measure of the input**. The input is the same:

| Area | Nodes (road edges): h3 | Nodes: tileroute | Arcs: h3 "Initial Shortcuts" | Arcs: tileroute |
|---|---|---|---|---|
| Somerset | 7,024 | 7,024 | 19,618 | 19,618 |
| Burnaby | 40,825 | 40,824 | 111,513 | 111,513 |
| Vancouver city | 75,779 | 75,779 | 225,932 | 225,932 |
| Metro Vancouver | 374,909 | 374,907 | 1,028,495 | 1,028,495 |

tileroute counts only the road edges that are in some arc: one road edge of Burnaby and two of Metro Vancouver have
no arc at all.

## Measured on the same input

All runs on 2026-11-18 on the same PC (WSL, 20 cores, 23 GB), one run at a time, driving, the same duckOSM databases.

**Making the shortcuts**

| Area | h3-routing-platform: time | h3-routing-platform: memory | tileroute: time | tileroute: memory | h3 shortcuts | tileroute shortcuts |
|---|---|---|---|---|---|---|
| Somerset | 24.4 s | 2.3 GB | 0.04 s | 63 MB | 527,396 | 36,760 |
| Burnaby | 61.5 s | 6.7 GB | 0.2 s | 123 MB | 4,708,820 | 241,245 |
| Vancouver city | 188 s | 11.4 GB | 1.5 s | 274 MB | 13,970,530 | 953,742 |
| Metro Vancouver | 953 s | 22.3 GB | 3.6 s | 873 MB | 61,501,123 | 3,026,705 |

- h3-routing-platform: its own settings (10 workers, HYBRID); memory = the sum of all its processes.
- tileroute: time = the shortcut steps (20 threads); memory = the peak of a whole run (read, shortcuts, 10,000 queries).

**Answering queries** (one thread, 10,000 random pairs, the same pairs for both)

| Area | h3 C++ `pruned`: per query | h3: load | h3: memory | tileroute: per query | tileroute: memory |
|---|---|---|---|---|---|
| Somerset | 2.5 ms | 0.2 s | 112 MB | 0.14 ms | 63 MB |
| Burnaby | 17.9 ms | 2.5 s | 598 MB | 0.63 ms | 123 MB |
| Vancouver city | 69.5 ms | 8.1 s | 1.5 GB | 2.5 ms | 274 MB |
| Metro Vancouver | 236.1 ms | 39.6 s | 6.5 GB | 5.2 ms | 873 MB |

- Both give the correct cost for all 10,000 pairs in each area. The h3 engine was checked with its own test program
  (`test_routing`) against the costs of duckOSM's router.
- h3: load = the time to read its shortcut table; memory = the peak of `test_routing`.
- tileroute: memory = the peak of the whole run, which also makes the shortcuts.

!!! info "What this comparison shows"
    The two programs use different shortcut rules: h3-routing-platform keeps shortcuts between all pairs of a cell, at
    each resolution, and computes them in SQL and Python; tileroute keeps only the shortcuts around each removed set and
    computes them in Rust. So the table compares the two methods as a whole. An exact Rust port of the h3 method was
    not made: with the top-down results ([The top-down phase](top-down.md)), it is not necessary for the plan.
