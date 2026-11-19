# The preparation tile by tile

!!! note "Status"
    `tileroute build --by-tile` (this page). The first version of `tileroute build` holds all of the network and all
    shortcuts in memory: Kentucky needs a peak of 4.5 GB, so France (26 million road edges) would need about 50 GB and
    Europe about 500 GB. This page describes the preparation that holds only one zoom-8 tile at a time.

## The idea

The steps of the hierarchy are local: the step of a set X (level z, cell T) reads and writes only the nodes of X, the
nodes next to X, and their arcs. A node of level 8 or finer lies in one zoom-8 tile, and it touches only nodes of the
same zoom-8 tile or nodes of level 7 and coarser (the **top**). So all steps of zoom 20 to zoom 8 of one zoom-8 tile
need only that tile and the top nodes next to it.

| Phase | Works on | Memory |
|---|---|---|
| 1. Nodes | all road edges: their level, cell, file and node number | about 25 bytes for each road edge |
| 2. Arcs into buckets | the `edge_graph` rows, as a stream; each arc goes to the bucket file of its zoom-8 tile, or to the top bucket | one row at a time |
| 3. Fine steps | one zoom-8 tile at a time: zoom 20 to zoom 8 | one tile |
| 4. Top steps | the top nodes: zoom 7 to zoom 1 | the top |
| 5. Files | one zoom-8 tile at a time: arc reduction, then its tile file; then the top file | one tile and the top |

## Phase 1: the nodes

For each road edge: its level (the zoom of the smallest tile that holds both of its ends) and its cell, as in
[Tiles and levels](levels.md). A node of level 8 or finer belongs to the file of its zoom-8 tile, a node of level 7 or
coarser to the top file. The node numbers are given now, before any step, in the order of the files (the top file
first, then the zoom-8 tiles), and in a file by level, then by cell. So each phase knows the final number of each node.

## Phase 2: the arcs into buckets

The rows of `<mode>.edge_graph` come as a stream. An arc with a node of level 8 or finer goes to the bucket of that
node's zoom-8 tile (if both nodes are fine, they are in the same tile). An arc between two top nodes goes to the top
bucket. The buckets are temporary files. If a pair comes more than once, the last row counts, as in duckOSM's router.

## Phase 3: the fine steps, one tile at a time

For one zoom-8 tile: read its bucket, make the local graph (the fine nodes of the tile and the top nodes that its arcs
touch), and run the steps of zoom 20 to zoom 8, as in [Shortcuts](shortcuts.md). The witness search of a step sees
only the local graph; a witness that it cannot see only keeps a shortcut that is not needed, so the routes stay exact.

The result goes to a temporary file of the tile: each arc with a node of this tile, with its saved inner nodes. The
shortcuts between two top nodes go to the top: the top keeps the cheapest one of each pair.

## Phase 4: the top steps

The top graph: the arcs of the top bucket and the shortcuts between top nodes from all tiles. Its steps are zoom 7 to
zoom 1, as before. The top is small (Kentucky: 3,340 nodes), so it stays in memory until the end.

## Phase 5: the files, one tile at a time

For one zoom-8 tile: read its result file, make its two search graphs (forward and backward) together with those of the
top, run the [arc reduction](shortcuts.md#arc-reduction-fewer-arcs-for-the-search) for its nodes, and write its tile
file. Then the top file, with its arcs to the fine nodes of all tiles, then `edges.idx` and `index.bin`. The format is
the same as before ([Tile files](../reference/tile-files.md)), so the query does not change.

## Correctness

| Point | Why it is correct |
|---|---|
| Fine steps of different tiles | they touch no common node except top nodes, which none of them removes |
| A witness that one tile cannot see | it only keeps a shortcut that is not needed |
| The same top pair from more than one tile | the top keeps the cheapest cost and its path |
| Arc reduction with the top | the search graph of a fine node goes only to its tile and to the top |

The check: `tileroute check <db> --files <folder>` compares the routes from the files with the routes in memory and
with duckOSM's router.

## Results (Kentucky, WSL PC)

| | `tileroute build` (all in memory) | `tileroute build --by-tile` |
|---|---|---|
| Peak memory | 4.5 GB | **1.8 GB** |
| Time | about 26 s | 31 s |
| Files | 704 MB | 740 MB |
| Routes from the files | — | 2,000 of 2,000 equal to the routes in memory; 0 wrong against duckOSM's router (Monaco: all 7,645,225 pairs) |
| One query from the files (one thread) | — | 1.3 ms (WSL PC), 3.7 tiles and 3.0 MB read |

| Phase (Kentucky) | Time |
|---|---|
| 1. Nodes | 1.1 s |
| 2. Arcs into buckets | 2.4 s |
| 3. Fine steps, 18 tiles one at a time | 12.1 s |
| 4. Top steps (3,340 nodes) | 2.9 s |
| 5. Files: arc reduction of each tile together with the top | 12.6 s |

The files are 5% larger: a tile cannot see the witnesses of other tiles, and the arc reduction of a tile uses the top
before the top's own reduction, so a few more shortcuts and arcs stay.

## Results: France (Mac mini, 16 GB)

France driving, from `france-latest.osm.pbf` (2026-11-19). duckOSM's database is 28.5 GB, so `tileroute slim` first copies
only the three tables that tileroute reads (2.0 GB, 5 s).

| | Value |
|---|---|
| Road edges (nodes) | 25,919,934 (28,192 in the top) |
| Arcs | 62,517,049 |
| Zoom-8 tiles | 72 |
| `tileroute build --by-tile` | **547 s, peak memory 9.7 GB** (all in memory: about 50 GB, so not possible on this Mac) |
| Tile files | 8.7 GB; the top file 794 MB, the largest tile 340 MB |

| Phase | Time |
|---|---|
| 1. Nodes | 7 s |
| 2. Arcs into buckets | 32 s |
| 3. Fine steps, 72 tiles one at a time | 129 s |
| 4. Top steps (28,192 nodes) | 256 s |
| 5. Files | 121 s |

**Queries from the France files** (one thread, 2,000 random pairs, `tileroute route --random 2000`):

| | Value |
|---|---|
| One query | **5.3 ms** (the files in the cache of the system); 9.5 ms on the first pass |
| Read for each query | 7.2 tiles, 31 MB |
| Peak memory of the process | 4.3 GB, almost all pages of the mapped files |
| duckOSM's router today (same Mac) | load 11 to 16 s, one route Paris to Marseille 3.7 s, peak 6.4 GB |

Not checked yet for France: the routes against duckOSM's router (the check needs all of France in memory). The method
is the same as on Kentucky and Monaco, where all checked routes are equal.

**What to improve next:** the top. Its 28,192 nodes take 256 s to prepare, and its file (794 MB) is most of the 31 MB
that a query reads.

**What decides the peak memory now:** the largest zoom-8 tile (Kentucky: Louisville, 668,580 nodes) and the node tables
(about 25 bytes for each road edge). For France, the largest tile (Paris) has more nodes than Louisville; the peak is
expected to fit in the 16 GB of the Mac mini, but it is not measured yet. Europe (about 250 million road edges) needs
about 6 GB for the node tables alone; a next step can keep them on disk too.
