# Cells from the graph (the partition version)

!!! note "Status"
    An option, not the default: `tileroute check --partition 16` and `tileroute build --by-tile --partition 16`.
    Measured on Monaco, Kentucky and France. On France the queries are about 10 times faster than with the XYZ tiles
    ([Results: France](#results-france)).

This page describes the partition version: the same method as the [XYZ-tile cell version](levels.md), but the cells come from
a partition of the road graph, not from the XYZ tiles. It is step D1 of
[The whole world](distributed.md).

## The problem that it solves

In the XYZ-tile cell version, the cell of a node is an XYZ tile, so the borders between cells are straight lines. A set (the
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

| | XYZ-tile cells | Partition version |
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

## The three versions

Two choices make a version: the **order** (where the cells of the hierarchy come from) and the **files** (how the
nodes are grouped on disk). The query reads all three versions with the same code.

| Version | Order: the cells | Files | Build command |
|---|---|---|---|
| **1. XYZ-tile cells** (the default) | XYZ tiles, straight borders | one file for each zoom-8 XYZ tile (`8_x_y.trt`) and the top (levels 0 to 7) | `build --by-tile` |
| **2. Partition cells, cell files** | the partition of the road graph | one file for each partition cell of level 3 (`3_x_y.trt`, a cell key, not a map tile) and the top (levels 0 to 2) | `build --by-tile --partition 16` |
| **3. Partition cells, XYZ tile files** | the partition of the road graph | one file for each zoom-8 XYZ tile (`8_x_y.trt`), the same tiles as duckOSM and PMTiles, and the top (levels 0 to 2) | `build --by-tile --partition 16 --xyz-files` |

**Kentucky, the same WSL PC, the same 10,000 pairs, one thread, from the files:**

| | 1. XYZ-tile cells | 2. Partition cells, cell files | 3. Partition cells, XYZ tile files |
|---|---|---|---|
| Build tile by tile | 31.6 s | 16.1 s | 15.7 s |
| Peak memory of the build | 1.6 GB | 0.9 GB | 0.9 GB |
| Top nodes | 3,340 | 1,426 | 1,426 |
| Files | 18 + top, 706 MB | 64 + top, 419 MB | 18 + top, 419 MB |
| One query | 1.25 ms | 0.169 ms | **0.165 ms** |
| Files read for each query | 3.7 | 7.8 | **4.6** |
| Bytes read for each query | 3.0 MB | 192 KB | 192 KB |
| Routes from the files = routes in memory | 2,000 of 2,000 | 2,000 of 2,000 | 2,000 of 2,000 |

**What makes the difference:**

- **The order decides the speed.** Versions 2 and 3 have the same order, so they read the same bytes and take the same
  time. Version 1 has the straight tile borders, so its top is dense: 7 times slower on Kentucky, 10 times on France.
- **The files decide only how many files a query opens.** Zoom-8 tiles are larger than the level-3 cells of Kentucky,
  so version 3 opens fewer files than version 2.
- **Version 3 keeps the project's tiling** for the files: a file has a standard z/x/y name, as in duckOSM, pbfcut and
  PMTiles. A file is valid only with the other files of the same build, because a shortcut depends on the partition
  cell where it was made, and a cell can cross a tile border.

**France** (Mac mini): version 1: 5.32 ms warm, 10.3 ms cold; version 2: 0.54 ms warm, 1.35 ms cold
([Results: France](#results-france)). Version 3 is not measured on France yet; on Kentucky it is equal to version 2.

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

The order of the nodes and the files are two separate choices. The query needs only "node → file" (one range of node
numbers for each file), so the files do not have to be the cells of the order.

| Files | Option | Kentucky (the same 10,000 pairs, one thread) |
|---|---|---|
| By partition cell: one file for each cell of level 3 (64 files) and the top | `build --partition 16` | 0.172 ms for each query, 7.8 files and 192 KB read |
| **By XYZ tile:** one file for each zoom-8 tile (`8_x_y.trt`, 18 files), the same tiles as duckOSM and PMTiles, and the top | `build --partition 16 --xyz-files` | **0.172 ms** for each query, **4.6 files** and 192 KB read |

The file of a road edge is the zoom-8 XYZ tile of its first junction. The top file holds the nodes of level 2 and
coarser (`--top Z` changes this). So the files keep the project's tiling, and the speed comes from the partition. The
routes from the XYZ files are equal to the routes in memory (2,000 of 2,000).

A tile file is valid only together with the other files of the same build: a shortcut depends on the partition cell
where it was made, and a cell can cross a tile border.

`--xyz-files` works with the build in memory and with `build --by-tile`. The build tile by tile still runs the steps
one partition cell (of level 3) at a time; after the arc reduction of a cell, the arcs of each node go to a temporary
bucket of its XYZ tile, and then each XYZ tile file is written from its bucket. The format of the files is the same
([Tile files](../reference/tile-files.md)).

## Results: Kentucky

**Routes.** Monaco: all 7,645,225 pairs equal to duckOSM's router. Kentucky: 300 random pairs, 0 wrong. The routes
from the files are equal to the routes in memory (2,000 of 2,000).

**The same computer (WSL PC), the same 2,000 pairs, one thread:**

| | 1. XYZ-tile cells | 2. Partition cells (leaf 16) |
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

## Results: France

Mac mini (16 GB), 2026-11-19, `bench/francebench.sh` with `TRARGS="--partition 16"`. The same `.osm.pbf` and the same
2,000 pairs as the XYZ-tile cell version ([Comparison with other routers](engines.md#measured-on-france)). One thread.

| | 1. XYZ-tile cells | 2. Partition cells, cell files |
|---|---|---|
| Top nodes | 28,192 (level 7 and coarser) | 4,184 (level 2 and coarser) |
| `build --by-tile`, in all | 547 s | 545 s |
| of this: cells (the partition) | — | 478 s |
| of this: steps below the top | 129 s | 15 s |
| of this: steps of the top | 256 s | 0.8 s |
| of this: arc reduction and files | 121 s | 16 s |
| Peak memory of the build | 9.7 GB | 8.4 GB |
| Files | 8.1 GB (72 zoom-8 files and the top) | 4.7 GB (64 level-3 files and the top) |
| One query, warm (second pass) | 5.32 ms | **0.54 ms** |
| One query, cold (first pass) | 10.3 ms | **1.35 ms** |
| Read for each query | 7.2 files, 31 MB | 7.0 files, 1.5 MB |
| Memory of the query process | 4.3 GB | 2.1 GB |
| Pairs with a route | 1,939 of 2,000 | 1,939 of 2,000 |

On the same Mac, OSRM CH needs 1.15 / 1.48 ms (warm / cold) and GraphHopper CH (no turn costs) 0.89 / 1.17 ms.

**Now the partition is the slow part of the build** (478 s of 545 s): the first cuts are large maximum-flow problems
on all 11 million junctions, and only the 4 lines of a cut run in parallel there. The steps themselves take 32 s.

Not checked for France: the routes against duckOSM's router (the check needs all of France in memory), and the costs
pair by pair against the XYZ-tile cell version. Monaco and Kentucky are checked (0 wrong).

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
coarse levels, also inside a region: the slow part of the XYZ-tile cell version is inside one region (France alone: 5.3 ms for
each query, 28,192 top nodes), and Kentucky, which is smaller than one region, got 7.8 times faster only from the cuts
inside it.

## Comparison with CCH and MLD

All three use nested cells with small cuts. OSRM MLD also uses inertial flow. The differences are in what each one
builds on the cells, and in how the query uses them.

| Point | CCH (customizable contraction hierarchy) | MLD (OSRM MLD, CRP) | tileroute, partition version |
|---|---|---|---|
| Graph | node-based (turns need an extra step) | node-based, with turn tables at the junctions | edge-based: a node is a road edge, so a turn is a normal arc |
| Cuts | node separators (junctions) | edge cuts (cut roads) | edge cuts of the junctions; the cut roads are the nodes of a set |
| Levels | deep: one level for each cut | few: about 4 to 5 levels of cells | deep: about 13 levels with 4 children (two cuts) each |
| Order inside a separator | each node has its own rank (a total order) | no order: the border nodes of a cell make a clique | all nodes of one set have the same rank and are removed together |
| How the shortcuts are made | one node at a time, with no witness search | a clique between all border nodes of each cell | one Dijkstra through the whole set, with a witness search |
| Depends on the costs | no: the structure is fixed; the costs come later in a separate customization step | no: the cliques get their costs in the customization | yes: the witness search uses the costs |
| New costs (traffic) | customization: some seconds for Europe | customization: some seconds for each level | the steps run again (Kentucky: 4 s; France not measured) |
| Arcs that are not needed | removed by the perfect customization | stay (full cliques) | removed by the witness search and the arc reduction |
| Query | elimination-tree search (no priority queue), or bidirectional Dijkstra up only | bidirectional Dijkstra that changes level by the cells of the start and the end | bidirectional Dijkstra up only, as in a CH |
| Unpack of the path | from the triangles of the shortcuts | a search in each cell again, or stored paths | the saved inner nodes of each shortcut |

**The main differences:**

1. **Sets, not single nodes.** tileroute removes a whole set with one Dijkstra. CCH removes one separator node at a
   time. The steps of tileroute are simple and run in parallel, but the search moves freely inside a set. This is not
   a problem when the sets are small.
2. **The costs are in the structure.** CCH and MLD make the structure once with no costs, then add the costs fast
   (customization). tileroute uses the costs in the witness search, so new costs run the steps again. This agrees with
   the decision to rebuild all costs on a schedule ([Updates](updates.md)).
3. **Edge-based graph.** In tileroute a turn cost is an arc. CCH and MLD need more work for turns.
4. **Saved paths.** The unpack is fast, but the saved paths use memory.

**What tileroute can take from them** (the CCH idea is now measured: [A structure with no costs](customization.md)):

| From | Idea | Gain | Cost |
|---|---|---|---|
| CCH | No witness search: a structure that does not depend on the costs, then a fast customization for new costs | fast updates for traffic and for the weights of each query ([Weights](weights.md)); tileroute already keeps every shortcut pair, so it is near this | more arcs, so slower queries unless a perfect customization removes them |
| MLD | Fewer and larger levels | a smaller overlay for [the whole world](distributed.md) | more work inside each cell for each query |

## Advantages and disadvantages

| | XYZ-tile cells | Partition version |
|---|---|---|
| Query speed and arcs | slower; the top grows with the area | faster; the cuts stay small |
| Preparation | no partition | the partition first (Kentucky: 7 s); the steps are faster |
| Keys | the same for each version of the data | can change when the roads change |
| Two areas joined | their tiles fit together | the partition must run again on the joined area |
| Point to cell | from the coordinates | from a tile index (point → road edge → cell) |

## Next steps

| Step | Done means |
|---|---|
| A faster partition (the first cuts in parallel, or fewer junctions with a coarse graph first) | France: the cells in less than 100 s |
| The mix (tiles inside small cells) | Kentucky: the same table, next to the partition version |
| The partition as the default | after France, if the result is as on Kentucky |
