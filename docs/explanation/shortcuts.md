# Shortcuts

## What a shortcut is

A shortcut is an arc that tileroute adds to the graph. A shortcut `u → w` has the cost of a shortest path from `u` to
`w`, so a search can go from `u` to `w` in one step. The nodes between `u` and `w` are not on the shortcut; tileroute
finds them again when it writes the path ([The shortest path](query.md#unpack)).

## How tileroute makes the shortcuts

tileroute goes from the finest level (20) to zoom 1. At each zoom `z`, it does one step for each tile `T` of that zoom:

1. **The set X:** the nodes of level `z` with cell `T`. These are the road edges that cross the borders between the
   children of `T`. The road edges of finer levels in `T` are already removed (at finer zooms).
2. **The nodes u:** each node that is not removed and not in X, with an arc into X.
3. **The nodes w:** each node that is not removed and not in X, with an arc out of X (`w` ≠ `u`).
4. **The shortcut u → w:** its cost is the cost of the shortest path from `u` to `w` whose inner nodes are all in X.
   tileroute runs one Dijkstra from each `u`, only on the nodes of X.
5. **Keep it** only if there is no arc `u → w`, or if the shortcut is cheaper than that arc.
6. **Remove X.** The later steps do not use the nodes of X. Their arcs and the shortcuts stay in the graph for the
   query.

A node `u` or `w` is not removed yet, so its level is coarser than `z`. It touches `T` (it has a common junction with a
node of X), so it is a road edge that **crosses the border of T**: one end in `T`, the other end outside.

## Example

Tile `T` at zoom 16. The nodes of level 16 in `T` are `x1`, `x2` and `x3`. The streets inside the children of `T` (level
17 to 20) are already removed. Two coarser roads come into X (`u1`, `u2`), and two go out (`w1`, `w2`).

```mermaid
flowchart LR
    u1([u1: in]) --> x1
    u2([u2: in]) --> x2
    subgraph T [tile T, zoom 16: the set X]
        x1 --> x2
        x2 --> x3
        x1 --> x3
    end
    x3 --> w1([w1: out])
    x2 --> w2([w2: out])
```

The step makes at most 2 × 2 = 4 shortcuts: `u1 → w1`, `u1 → w2`, `u2 → w1`, `u2 → w2`. Each one has the cost of the
shortest path through X. Then `x1`, `x2` and `x3` are removed.

## Why both ends of a shortcut are outside X

A shortest path that goes **through** X comes in from some `u` and goes out to some `w`. The shortcut `u → w` gives
that part of the path in one step. So the shortcuts of all steps together let a search jump over each tile that the
path only goes through.

A path that **starts or ends** in X does not need a shortcut from an inner node. The original arcs between the nodes of
X stay in the graph, and the search walks on them to the border of X ([The shortest path](query.md)).

A shortcut with one end inside X (`x → w`, as in h3-routing-platform) is also possible. Then a search can leave the
start cell in one step, but the number of shortcuts grows by about (nodes in X) × (border nodes) for each cell.
[Comparison with h3-routing-platform](comparison.md) shows the difference.

## Why the tiles of one zoom run in parallel

Two sets X of the same zoom have no common node and no arc between them: their road edges are in different tiles and
have no common junction. A step reads the graph and writes only new shortcuts, so tileroute runs all tiles of one zoom
on all threads, then adds their shortcuts.

## The result for each zoom (Kentucky, driving)

| Zoom | Nodes of this level | Shortcuts made |
|---|---|---|
| 20 | 250,780 | 577,152 |
| 17 | 376,176 | 1,148,536 |
| 14 | 160,255 | 927,767 |
| 11 | 26,537 | 655,667 |
| 8 | 3,241 | 334,815 |
| 6 | 360 | 60,138 |
| 5 | 973 | 0 (the top: no step) |

Kentucky has 2,149,438 nodes and 5,645,321 arcs. tileroute makes 10,318,657 shortcuts in 13.8 s
([Measurements](../reference/measurements.md)).

!!! info "The name of this method"
    This is a contraction hierarchy (as in OSRM and GraphHopper) whose order comes from the tiles: a node of a lower
    level is higher in the hierarchy. A plain contraction hierarchy removes one node at a time; tileroute removes a
    whole set X at a time.
