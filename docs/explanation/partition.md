# Cells from the graph (the partition version)

!!! note "Status"
    An option, not the default: `tileroute check --partition 16` and `tileroute build --by-tile --partition 16`.
    Measured on Monaco and Kentucky. France is not measured yet. The XYZ tile version stays the default until France
    is measured.

This page describes the partition version: the same method as the [tile version](levels.md), but the cells come from
a partition of the road graph, not from the XYZ tiles. It is step D1 of
[The whole world](distributed.md).

## The problem that it solves

In the tile version, the cell of a node is an XYZ tile, so the borders between cells are straight lines. A set (the
nodes of one level in one cell) holds every road that crosses the lines of its cell. All nodes of a set have the same
rank, so the search goes through a set as a plain Dijkstra does.

The fine sets are small, but the coarse sets are large. On France, the top (level 7 and coarser) has 28,192 nodes, and
a query opens about 2 million arcs (31 MB). For the world, the top is estimated at more than 50 GB.

A query must go through the separators between its start and its end. So the size of the separators decides the speed.
Straight lines are not good separators: they cut each road on their way. A cut from the graph goes where few roads
cross: a river with few bridges, a mountain, the edge of a city.

## Tiles and partition: one method, two cell functions

The method needs only two properties of the cells:

1. **Nested.** Each cell is in one parent cell.
2. **No overlap.** The cells of one level do not overlap.

With these two properties, two nodes of one level in different cells have no arc between them, and the sets of one
level are independent. A cell does not have to be a square. So the set steps, the witness search, the saved paths, the
arc reduction and the query do not change. Only the function that gives the level and the cell of a node changes.

| | Tile version | Partition version |
|---|---|---|
| Cell | an XYZ tile | a part of the road graph |
| Border | straight lines (longitude, Mercator latitude) | the cut with the fewest roads that the partition finds |
| Children of a cell | 4 | 4 (two cuts) |
| Cell key | a tile key: quadkey digits and zoom | the same form: 2 bits for each level and the level |
| Level of a road edge | the finest zoom where both ends are in one tile | the finest level where both ends are in one cell |
| Depends on | the coordinates only | the road graph and the coordinates |
| Preparation of the cells | none | the partition (Kentucky: 7 s) |

The cell key has the same form as a tile key, so the functions `parent` and `lca_zoom` of
[src/tile.rs](https://github.com/Khoshkhah/tileroute/blob/main/src/tile.rs) work unchanged for both versions.

## The partition: inertial flow

The partition cuts the **junctions** (the ends of the road edges). A road edge with its two ends in different parts is
in the cut. The code is in [src/part.rs](https://github.com/Khoshkhah/tileroute/blob/main/src/part.rs).

**One cut of a cell in two parts:**

1. Put the junctions of the cell in order along a line, from their coordinates (longitude × cos(mean latitude), and
   latitude).
2. Take the first 25% of the junctions as the source side and the last 25% as the target side.
3. Find the minimum cut between the two sides: Dinic's maximum flow, with capacity 1 for each road edge. The source
   part is all junctions that the rest graph reaches from the source side.
4. Do steps 1 to 3 for 4 lines: west-east, south-north and the two diagonals. Keep the smallest cut. If two cuts have
   the same size, keep the one with the better balance.

**One level:** cut the cell in two, then cut each half in two. The four parts get the digits 0, 1, 2 and 3. A cell
stops when it has at most LEAF junctions (`--partition 16`: at most 16), or at the finest level (20). The road edges
inside a stopped cell get the finest level.

**Parallel work:** the cells of one level are cut at the same time, and the 4 lines of one cut also run at the same
time.

**Only the network.** The partition uses the roads and the coordinates, not the costs. A new cost (traffic) does not
change it, so the plan to rebuild the costs on a schedule ([Updates](updates.md)) stays the same. The partition runs
again only when the road network changes.

## Files

`build --by-tile --partition 16` writes one file for each cell of level 3 (64 files at most) and one top file (levels
0 to 2). `--top Z` changes this: the top is level Z and coarser, and there is one file for each cell of level Z + 1.
The format of the files does not change ([Tile files](../reference/tile-files.md)), so the query does not change.

## Results: Kentucky

**Routes.** Monaco: all 7,645,225 pairs equal to duckOSM's router. Kentucky: 300 random pairs, 0 wrong. The routes
from the files are equal to the routes in memory (2,000 of 2,000).

**The same computer (WSL PC), the same 2,000 pairs, one thread:**

| | Tile version | Partition version (leaf 16) |
|---|---|---|
| Nodes of the coarsest level | 973 (zoom 5) | 120 (level 0) |
| Largest forward-search degree of a node | 238 | 39 |
| Shortcut steps, in memory | 37 s | 4.2 s, and 7.1 s for the partition |
| Arcs | 15.3 million | 7.2 million |
| Nodes settled by one query | 4,222 | 680 |
| Arcs opened by one query | 180,733 | 8,789 |
| One query, in memory | 1.79 ms | **0.23 ms** |
| Query data in memory | 747 MB | 447 MB |
| `build --by-tile`: time, peak memory | 31 s, 1.8 GB | 14.5 s, 0.9 GB |
| Files | 740 MB (18 zoom-8 files and the top) | 438 MB (64 level-3 files and the top) |
| One query from the files | 1.3 ms, 3.0 MB read | **0.17 ms, 194 KB read** |

**The cuts of each level (Kentucky),** in road edges: 120, 354, 952, 2,184, 5,256, 13,641, 37,187, 96,182, 133,631,
64,990, 11,863, 956, 10. The other 1,782,112 road edges are inside the leaf cells.

## What the tiles still do

| Use | Why the tiles stay |
|---|---|
| Find the road edge of a point | a tile index from a point to the road edges near it is fast and simple |
| The browser | it can find the files of an area from the tiles of that area |
| The finest levels (an option, not measured) | see [A mix](#a-mix-tiles-inside-small-cells) |

## A mix: tiles inside small cells

An option, not measured: the partition for the coarse levels, down to cells of a few thousand junctions; inside each
of these cells, the XYZ tiles. The key of a node is then the digits of its partition cell, followed by the digits of
its tile. The keys stay nested, so the method does not change.

| | Expected result |
|---|---|
| Speed | almost the same as the partition version: a tile of zoom 16 or finer cuts few roads |
| Local data | keeps tile keys inside the small cells |
| The coarse levels | small cuts, as in the partition version |

## Regions and the overlay

In [The whole world](distributed.md), a **region** is a cell of a coarse level of this partition (for example 20 to
50 million road edges), and the **overlay** is all nodes of the region level and coarser. The partition is used at all
coarse levels, also inside a region: the slow part of the tile version is inside one region (France alone: 5.3 ms for
each query, 28,192 top nodes), and Kentucky, which is smaller than one region, got 7.8 times faster only from the cuts
inside it.

## Advantages and disadvantages

| | Tile version | Partition version |
|---|---|---|
| Query speed and arcs | slower; the top grows with the area | faster; the cuts stay small |
| Preparation | no partition | the partition first (Kentucky: 7 s); the steps are faster |
| Keys | the same for each version of the data | can change when the roads change |
| Two areas joined | their tiles fit together | the partition must run again on the joined area |
| Point to cell | from the coordinates | from a tile index (point → road edge → cell) |

## Next steps

| Step | Done means |
|---|---|
| France on the Mac mini (`TRARGS="--partition 16"` in `bench/francebench.sh`) | preparation time, peak memory, query time warm and cold and MB read for each query, next to the tile version and the other routers |
| The mix (tiles inside small cells) | Kentucky: the same table, next to the partition version |
| The partition as the default | after France, if the result is as on Kentucky |
