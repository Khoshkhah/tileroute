# tileroute: design and plan

Status: first draft, 2026-11-18. The method changed after this draft: see docs/explanation/ (the site). This file is kept for its history.

## 1. The problem

duckOSM's router (`rust/duckosm-router`) loads all of `<mode>.edge_graph` into one CSR graph in memory. For France
driving (25.9 million edges, about 68 million turns) on the Mac mini (Apple M6, 12 cores, 16 GB):

| Measure | Today |
|---|---|
| Load time | 11 to 16 s |
| One route, Paris to Marseille | 3.7 s |
| Peak memory | 6.4 GB |

tileroute keeps the graph on disk, cut by XYZ map tiles. A query reads only the tiles that it needs. The same Rust code
runs natively (memory map) and in the browser (WebAssembly, range reads).

## 2. Input

tileroute reads one duckOSM database, one mode at a time.

| Table | Columns that tileroute uses |
|---|---|
| `<mode>.edge_graph` | `from_edge`, `to_edge`, `cost` (FLOAT), `uses` |
| `<mode>.edges` | `edge_id`, `geometry`, `cost_s`, `length_m` |

The graph is edge-based. A graph node is one directed road piece (`edge_id`, one id for each direction). An arc is one
legal turn. tileroute keeps this model. It reads the same rows as duckOSM `routed_graph` (default `uses`: the mode's
own use, for example `car`). Each other set of `uses` is a separate build.

**The rule of equality.** A route must equal the route of `duckosm-router` (`bidirectional_dijkstra`): the same
`edge_id`s and the same `time_s`.

- `cost` is f32. tileroute keeps each cost as the same f32 and adds costs in f64. An f32 has 24 bits of mantissa. If each
  cost that is not zero is at least 0.001 s (its last bit is at least 2^-34) and a sum is less than 2^17 s (36 hours),
  all bits of the sum fit in the 53 bits of an f64. Then each sum is exact, the order of the additions has no effect,
  and a shortcut cost is exactly the sum that networkx makes. The preparation checks these two limits and stops if a
  cost is out of them.
- When two different paths have exactly the same cost (a tie), networkx takes one by its heap order. A hierarchy can
  take the other one. See section 9, question 1.

## 3. The method

### 3.1 The choice

| Method | Used by | Preparation | Memory at query time | Reads tiles on demand | Exact shortest path | Browser |
|---|---|---|---|---|---|---|
| Plain bidirectional Dijkstra | duckOSM today | none | all of the graph | no | yes | too large for a country |
| Contraction Hierarchies (CH) | OSRM, GraphHopper | node order for all of the graph, hours for Europe | all of the graph (memory map is possible, but the search touches pages everywhere) | no | yes | no |
| Tiled graph with road-class levels | Valhalla | cut by tiles (4°, 1°, 0.25°) | the tiles on the way, LRU cache | yes | no: the levels are a heuristic | possible |
| Shortcuts by H3 cell, LCA pruning | h3-routing-platform | Python and DuckDB SQL, cell by cell | all shortcuts (C++ engine) | no | yes | no |
| Overlay by XYZ tile (this design) | tileroute | Rust, tile by tile, in parallel | the tiles of the query | yes | yes | yes |

tileroute takes the idea of h3-routing-platform: a hierarchy of cells, and a query that goes up from the source cell
and the target cell to their lowest common ancestor (LCA). In the literature this is a multi-level overlay graph
(Customizable Route Planning, OSRM's MLD), here on the quadtree of XYZ tiles.

Why this choice:

- It is exact, and it reads by tile. CH is exact but does not read by tile. Valhalla reads by tile but is not exact.
- XYZ tiles are the one tiling protocol of the stack (duckOSM builds, pbfcut, map tiles, PMTiles). Each tile has one
  parent and 4 children that cover it exactly, so the LCA of two tiles is the common prefix of their quadkeys.
- Each tile is prepared alone, so the preparation runs in parallel and needs little memory.
- A change of costs (for example SonoFlow flows) needs a new overlay only, not a new hierarchy.

The cost of the choice: tile borders do not follow the road network, so a tile has more border nodes than a cell of a
graph partitioner (OSRM uses PUNCH or KaHIP). More border nodes give larger overlays. M2 measures this.

### 3.2 Tiles and levels

| Term | Meaning |
|---|---|
| Leaf zoom | 14 (about 2.4 km x 1.7 km in France). Each graph node has one leaf tile. |
| Node tile | The leaf tile of the start point of the edge geometry, in the direction of travel. Same tile formula as pbfcut `tiles.rs`. |
| Levels | Zooms 6, 8, 10, 12, 14. One level is 2 zooms, so a level tile has 16 child tiles. M2 can change the levels. |
| Node key | u64: the leaf tile (x and y at zoom 14, 28 bits) and the node number in that tile (u32). |
| Border node of tile T | A node in T with a turn to or from a node out of T. An entry has a turn in; an exit has a turn out. |
| Cut arc of tile T | A turn from a node in T to a node out of T, or the opposite. |
| Overlay of tile T | For each entry and each exit of T: the shortest cost from the entry to the exit, with all of the path in T. |

### 3.3 The search

For a query from node `s` to node `t`, each node `v` gets a level:

- If `v` is in the leaf tile of `s` or of `t`: level "base". The search uses the turns of `v`.
- If not: the coarsest level zoom `Z` at which the tile of `v` holds neither `s` nor `t`. The search uses the overlay of
  that tile from `v`, and the cut arcs of that tile from `v`. If no level zoom is coarse enough, it uses zoom 6.

Near `s` and `t` the search uses fine tiles. Far from them it uses coarse tiles and jumps over them with the overlays.
The search is the same bidirectional Dijkstra as `duckosm-router` (heap keys `(distance, counter)`, the same stop
rule), on this smaller graph. The labels are in a hash map by node key, not in arrays of 26 million.

This is exact: a shortest path that goes through a tile which holds neither `s` nor `t` goes in at an entry and out at
an exit, and the overlay holds the shortest cost between them.

## 4. The files

### 4.1 Layout

| File | Holds |
|---|---|
| `tiles/8_<x>_<y>.trt` | One file for each zoom-8 tile (the tile size of duckOSM builds and pbfcut). It holds the sections of all level tiles of zooms 8, 10, 12, 14 in it. |
| `top.trt` | The sections of the zoom-6 level tiles. |
| `edges.idx` | All `edge_id`s, sorted, each with its node key. |
| `index.trt` | The header and the directory of the files above. |
| `<area>.<mode>.tileroute` | Optional archive, like PMTiles: the header, the directory, then all of the files above, one after the other. `tileroute pack` makes it. |

The build writes the folder. The browser and HTTP use the archive. The reader reads both.

### 4.2 One file

A `.trt` file has a header, a directory of its sections (sorted by zoom, then quadkey: offset u64, length u32), then the
sections. All numbers are little-endian and aligned to 8 bytes, so a memory map can read them with no decode step.

A **leaf section** (zoom 14) holds:

| Part | Type |
|---|---|
| `edge_id` of each node | i64[n] |
| Turns out, in the tile (CSR) | offsets u32[n+1], target u32 (local), cost f32 |
| Turns in, in the tile (CSR, for the backward search) | the same |
| Cut arcs out and in | local u32, other node key u64, cost f32, cut zoom u8 |
| Entries and exits | local u32 |
| Overlay | f64[entries x exits] (infinity: no path in the tile) |

A **level section** (zooms 6 to 12) holds:

| Part | Type |
|---|---|
| Border nodes | node key u64, sorted (the local number is the place in this list, u32) |
| Entries and exits | local u32 |
| Overlay | f64[entries x exits] |
| Cut arcs of the tile, out and in | local u32, other node key u64, cost f32 |

A cut arc is in the section of each level tile that it leaves. This repeats some arcs (at most 5 times), but a query then
reads no leaf section far from `s` and `t`.

The overlay costs are f64 because they are exact sums (section 2). M2 measures the size. If it is too large, an
overlay can keep only the entry-exit pairs whose path goes through no other border node.

### 4.3 The edge index

`edges.idx` holds the pairs (`edge_id` i64, node key u64), sorted by `edge_id`, in blocks of 4096. The header keeps the
first `edge_id` of each block (France: about 6300 values, 50 KB). A lookup reads one block: one page fault natively, one
range read in the browser.

### 4.4 Size (estimate, M3 measures)

France driving: 26 million nodes and 68 million turns give about 1.5 GB of leaf sections. The overlays and the edge index
add about 0.5 to 1 GB.

## 5. The query

| Step | What happens |
|---|---|
| 1. Find the nodes | `edges.idx` gives the node key of `from_edge` and `to_edge`. |
| 2. The LCA | The common prefix of the two leaf quadkeys. |
| 3. The search | Section 3.3. It reads, for each level below the LCA, the child tiles of the ancestor of `s` (and of `t`), and the coarse tiles that the search reaches out of the LCA. |
| 4. Unpack | Each overlay arc on the path becomes a path in its tile: a small Dijkstra in that tile on the next finer level, down to the leaf turns. This reads the tiles along the route. |
| 5. Output | The `edge_id`s from the leaf sections. `time_s` and `length_m` come from `<mode>.edges`, or from `cost_s` and `length_m` stored in the leaf section (no database in the browser). |

**Reads.** All reads go through one small trait, "read these bytes at this offset". It has three implementations:
memory map (native), OPFS file (browser storage, in a worker), HTTP range request (browser, from a server).

**Cache.** Natively the memory map uses the cache of the operating system; tileroute keeps no cache of its own. In the
browser an LRU cache keeps the section bytes (default 64 MB).

**Memory.** The labels of one long query are a few hundred thousand nodes (about 30 MB). The sections that one query
reads are about 50 to 150 MB for Paris to Marseille (estimate).

## 6. The preparation

There is no Python in the pipeline. The `tileroute prepare` command does all of it.

| Step | Tool | What it does |
|---|---|---|
| 1 | DuckDB SQL, from Rust (`duckdb` crate) | Read `edges` and `edge_graph`. Give each node its leaf tile (`ST_StartPoint`), its node key and its zoom-8 tile. Mark each turn with its cut zoom (the common prefix of the two leaf quadkeys). |
| 2 | DuckDB SQL | Write the nodes and turns to Parquet, one part for each zoom-8 tile (`COPY ... PARTITION_BY`), sorted by node key. |
| 3 | Rust, one job for each zoom-8 tile, in parallel (`rayon`) | Leaf overlays: for each leaf tile, one Dijkstra from each entry, in the tile. Then zooms 12, 10, 8: the same on the child overlays and the cut arcs between the children. Write `8_x_y.trt`. |
| 4 | Rust | Zoom 6 from the zoom-8 overlays. Write `top.trt`, `edges.idx`, `index.trt`. |
| 5 | Rust (optional) | `tileroute pack`: the archive. |

h3-routing-platform makes its shortcuts with SQL self-joins, and changes to scipy for the coarse levels because the
joins need too much memory. Here SQL prepares the tables, and Rust runs the shortest paths.

The memory of one job is the size of one zoom-8 tile (about one duckOSM tile build). The disk space of the parts is
about the size of the database.

## 7. The checks

| Check | Area | Pass |
|---|---|---|
| Equality | Monaco (2765 edges): all pairs | Each route: the same `edge_id`s and the same `time_s` as `duckosm-router`. A different path is allowed only on an exact tie (equal cost), and the check reports each tie. |
| Equality | Kentucky driving: 10 000 random pairs, plus 100 long pairs | The same. |
| No path | Monaco, Kentucky | When `duckosm-router` finds no path, tileroute finds no path. |
| Files | Monaco | The folder and the archive give the same routes. Native and WebAssembly give the same routes. |
| Scale | France driving, on the Mac mini, only when Kaveh says so | Paris to Marseille and 1000 random pairs. Report peak memory, query time (cold and warm), preparation time and disk size, next to today's router (11 to 16 s load, 3.7 s, 6.4 GB). |

The reference routes come from `duckosm-router` (Rust crate on duckOSM `main`), called from the test code. Heavy runs go
to the Mac mini (`~/remote`, keep 30 GB free).

## 8. The plan

| Step | Work | Done means |
|---|---|---|
| M1 Monaco prototype | Crate `tileroute` (library and CLI, layout like pbfcut). `prepare` and `route` on the folder format, native memory map. | Monaco all pairs pass section 7. CI runs the Monaco check. |
| M2 Kentucky and levels | Kentucky. Measure overlay sizes and query reads for some level choices (for example 6-8-10-12-14, 8-11-14). Choose the levels. | Kentucky passes section 7. A table of sizes and times for each level choice, and the choice in this document. |
| M3 France | Archive format, edge index, parallel preparation. Python wrapper (only calls Rust). | France prepared on the Mac mini. The scale row of section 7 is measured and written in the README. |
| M4 Browser | WebAssembly package, OPFS and HTTP range reads, LRU cache, a demo page like pbfcut's (pick two points on a map, draw the route). | The page routes on Monaco and Kentucky from an archive, with the same routes as native. |

## 9. Questions for Kaveh

1. **Ties.** On an exact tie, the path can differ from networkx's path, with the same cost. Is that accepted (with a
   count of ties in each check)? To copy the heap order of networkx exactly, the search must see all nodes, which is
   the memory problem again.
2. **Repository name**: `Khoshkhah/tileroute`, MIT license, like pbfcut?
3. **Points as input** (nearest edge to a point): add in M4, or later?
