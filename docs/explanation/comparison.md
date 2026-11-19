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

| Area | h3 time | tileroute time | h3 memory | tileroute memory | h3 shortcuts | tileroute shortcuts |
|---|---|---|---|---|---|---|
| Somerset | 34 s ¹ | 0.09 s | — | 59 MB | 527,396 | 36,760 |
| Burnaby | 61.5 s ² | 0.25 s | 6.7 GB ² | 124 MB | 4,708,820 | 241,245 |
| Vancouver city | 4 min 56 s ¹ | 1.2 s | — | 282 MB | 13,970,530 | 953,742 |
| Metro Vancouver | 21 min 13 s ¹ | 3.9 s | — | 917 MB | 61,501,123 | 3,026,705 |

¹ From the logs of h3-routing-platform (January 2026). These logs give the memory of the main process only, not of its
10 workers, so the memory is not shown.
² Measured on 2026-11-18 on the same PC as tileroute (20 cores), with the settings of its Burnaby config (10 workers,
HYBRID). The memory is the sum of all processes.

!!! warning "What this table does not show"
    The two programs do different work: h3-routing-platform computes many more shortcuts. So the table compares the two
    methods as a whole, not the same algorithm in two languages. The next step is to port h3-routing-platform's exact
    method to Rust, check that it gives the same 4,708,820 shortcuts on Burnaby, and then compare the time, the memory
    and the query speed of the two shortcut rules on the same graph.
