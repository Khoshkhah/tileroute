# Comparison with other routers

!!! note "Two kinds of numbers on this page"
    The section [Measured on the same computer](#measured-on-the-same-computer) has our own measurements: all routers
    on one Mac mini, the same area, the same pairs. The other tables come from the papers and documentation of each
    router and show only the order of size.

## Measured on the same computer

Mac mini (Apple, 12 cores, 16 GB), 2026-11-19, one router at a time, nothing else running. Kentucky, from the same
`kentucky-latest.osm.pbf`; tileroute reads the duckOSM database that duckOSM builds from it. The same 10,000 pairs for
all routers: the start point of a road edge and the end point of another road edge (`bench/macbench.sh`).

| Router | Version | Preparation | Peak memory (preparation) | Disk | One query (one thread) | Memory at query time |
|---|---|---|---|---|---|---|
| OSRM CH | 26.11 | extract 16.8 s + contract 60.2 s | 5.4 GB | 1.0 GB | 0.58 ms | 796 MB |
| OSRM MLD | 26.11 | extract 16.8 s + partition 6.4 s + customize 1.7 s | 5.4 GB | 1.1 GB | 0.83 ms | 916 MB |
| GraphHopper CH | 11.1 | import 20.8 s (CH and Landmarks together) | 3.8 GB | 200 MB | **0.38 ms** | 807 MB (Java) |
| GraphHopper Landmarks | 11.1 | the same import | | | 3.7 ms | 807 MB (Java) |
| Valhalla | 3.9.3 | tiles 9.2 s | 2.8 GB | 258 MB | 5.2 ms | 614 MB |
| **tileroute**, first version | 0.1 | read 1.5 s + shortcuts 6.1 s | 5.2 GB ¹ | (no files yet) | 5.3 ms | not measured ² |
| **tileroute**, 2026-11-19 (saved paths, witness search, memory layout) | 0.1 | read 1.6 s + shortcuts 8.9 s (slower: the witness search) | 4.3 GB ¹ | (no files yet) | **1.38 ms** | not measured ² |
| **tileroute**, with arc reduction, in memory | 0.1 | read and shortcuts about 26 s | 4.5 GB ¹ | — | **0.66 ms** | not comparable ² |
| **tileroute**, with arc reduction, from tile files | 0.1 | the same, then write the files 0.3 s | 4.5 GB ¹ | **704 MB** | **0.81 ms** | **702 MB** ³ |

- The router queries go over HTTP on the same computer, one at a time with one kept connection: their time includes a
  small HTTP cost (about 0.1 ms). tileroute is called directly. Memory at query time: the server process after the
  10,000 queries.
- ¹ The peak of the whole run (read, shortcuts, queries).
- ³ A process that only maps the tile files and answers the 10,000 queries (`tileroute route`): its peak resident memory. Almost all of it is pages of the mapped files (the process has 9.1 MB before it reads them); the system can free these pages again. A query reads 3.7 tiles and 3.0 MB on average.
- ² Not comparable: this process also made the shortcuts. Its query data (computed size) is 1,169 MB in the first
  version and 802 MB now (hierarchy 750 MB + edge ids 52 MB), but the measured process holds 3.2 GB after the
  preparation, with the leftovers of the preparation and duckOSM's reference graph. A fair measurement needs a process
  that only loads a finished hierarchy (milestone M3). M3 has this process now ([Tile files](../reference/tile-files.md)),
  measured on the WSL PC, not yet on the Mac mini: 793 MB of files; after the 2,000 pairs, 764 MB resident, of which
  745 MB are pages of the mapped files and 1.4 MB own memory; 2.6 ms for one query (in memory on the same PC: 2.4 ms).
- The preparation of tileroute starts from the duckOSM database, so the work of reading the `.osm.pbf` (OSRM's extract,
  GraphHopper's import) is not in its number; duckOSM's build of Kentucky took 29 s for that work.

**Travel times.** Each router has its own road graph and speed model, so the routes are not equal to duckOSM's. The
table shows the travel time of each router divided by the time of duckOSM's router (the same as tileroute's), for the
9,664 pairs where duckOSM has a route:

| Router | Median ratio | Within ±10% | Within ±25% | Pairs with no route |
|---|---|---|---|---|
| Valhalla | 1.016 | 98.3% | 99.8% | 0 |
| GraphHopper (CH and Landmarks equal) | 1.135 | 16.1% | 92.1% | 89 (a point not found on its graph) |
| OSRM (CH and MLD equal) | 1.227 | 3.0% | 71.9% | 0 |

OSRM's car profile drives more slowly than duckOSM's speeds; Valhalla's speeds are near duckOSM's.

## Why tileroute is slower and larger today

Where the time of one tileroute query goes (Kentucky, the same pairs, one thread, `tileroute check --profile`):

| Part | Value |
|---|---|
| Measure | First version | With saved paths, flat arrays, stall-on-demand |
|---|---|---|
| Nodes that the search settles | 4,226 | 4,181 (a good contraction hierarchy: some hundreds) |
| Arcs that the search opens | — | **581,951: 139 for each settled node** |
| Search | 7.4 ms | 6.9 ms (WSL PC) |
| Unpack (shortcuts back to road edges) | 3.1 ms | 0.16 ms |
| One query | about 10.5 ms | 7.3 ms (WSL PC) |
| Query data in memory | 1,168 MB | 925 MB |

The saved paths remove the unpack cost. The **memory layout** then gives the next big gain (Kentucky, 2,000 pairs, one
thread, the same algorithm and the same arcs):

| Memory layout | One query |
|---|---|
| Node numbers in the order of the database rows | 7.3 ms |
| Node numbers by level, the top first (the nodes that a query reads most are together) | 4.0 ms |
| + the search arcs as one array of (cost, node) | 3.2 ms |
| + no stall-on-demand (its extra reads cost more than they save) | 2.35 ms |
| + arc reduction (remove search arcs that another path makes not needed: 543,097 → 180,380 arcs opened) | **1.09 ms** |

The query data in memory went down to 802 MB.

**Degree of the nodes** (Kentucky): on average 7.1 arcs leave each node of the shortcut graph, but the top nodes have
hundreds:

| Zoom | Nodes | Mean out-degree | Mean degree in the forward search |
|---|---|---|---|
| 5 to 7 | 3,340 | 320 to 383 | 147 to 178 |
| 8 | 3,241 | 164 | 103 |
| 9 to 12 | 96,267 | 21 to 84 | 13 to 52 |
| 13 to 20 | 2,046,590 | 2.6 to 12.8 | 2.6 to 7.8 |

So a query still opens about 540,000 arcs, 85% of them at zoom 5 to 8. A different order at the top would cut that
number; the memory layout makes each arc cheap.

| Cause | Effect | Fix |
|---|---|---|
| The order comes from the tiles only, and there is no witness search | the top of the hierarchy is dense (zoom 6: 137,928 shortcuts for 360 nodes), so the search opens many arcs | a witness search (skip a shortcut when a path of the same cost exists without it); important roads higher inside each set |
| Each shortcut was unpacked with a Dijkstra in its tile | 3.1 ms for each query | **done**: each shortcut saves its inner nodes (on average 1.7); unpack 0.16 ms |
| Labels in hash maps; node numbers in the order of the database rows | slower steps, cache misses | **done**: flat label arrays, node numbers by level, search arcs in one array: 7.3 → 2.35 ms |
| Each arc is stored three times, 24 bytes each (all, up, down) | 1.1 GB for Kentucky | one arc list with indices; a small tile number in place of the 64-bit key |

These fixes are milestone M2c of the [Plan](../plan.md).

## Road routers

| Router | Method | Preparation | One query | New costs (traffic) | Per-query weights | Reads by tile |
|---|---|---|---|---|---|---|
| OSRM (CH) | Contraction Hierarchies, nodes ordered by their importance, with a witness search | hours for Europe | below 1 ms (search), some ms with the path | a full preparation again | no | no: all in memory (or one memory-mapped file) |
| OSRM (MLD) | Customizable Route Planning: cells from a graph partitioner, cost tables between border nodes | partition one time; customization seconds to minutes | some ms | customization only | no (one profile for each build) | no |
| Bing Maps | Customizable Route Planning | as MLD | ms | customization: seconds | profiles | no |
| GraphHopper | Contraction Hierarchies (fixed), or Landmarks (A* with distances to some nodes, made one time) | CH as OSRM; Landmarks faster | CH: ms; Landmarks: tens of ms | Landmarks keep working with new costs | yes ("custom models", with Landmarks) | no |
| Valhalla | Tiles with three road-class levels, A* | the cut into tiles | tens to hundreds of ms for long routes | yes, traffic for each tile | yes (costing options) | yes, on demand, with a cache |
| Google Maps | not public | | | live traffic | | |
| **tileroute** | Shortcuts by XYZ tile (a contraction hierarchy ordered by tiles), exact | Kentucky about 26 s (Mac mini) | Kentucky 0.66 ms in memory, 0.81 ms from tile files (Mac mini, one thread) | a rebuild of all costs (seconds) | planned, exact (hull corners, M5) | yes: one file for each zoom-8 tile, memory-mapped on demand (M3) |

## Transit routers

| Router | Method |
|---|---|
| OpenTripPlanner | RAPTOR, with walking on the street graph |
| MOTIS, Navitia | RAPTOR and Connection Scan variants |
| Google Maps (transit) | Transfer Patterns (a heavy preparation) |
| **tileroute (plan)** | RAPTOR and McRAPTOR, joined to the walking hierarchy at the stops ([Timetable](timetable.md)) |

## Where tileroute is strong

- **Exact routes**, equal to duckOSM's router. The road-class levels of Valhalla do not always give the shortest path.
- **One tiling protocol**: the same XYZ tiles as the map tiles, duckOSM and pbfcut.
- **A structure that does not depend on the costs**: a rebuild with new costs only computes the costs again, as in MLD.
- **Per-query weights, exact** (planned): GraphHopper gives per-query weights with Landmarks, which are slower.

## Where tileroute is weak today

| Weak point | Reason | Possible fix |
|---|---|---|
| Queries slower than OSRM CH and GraphHopper CH (Mac mini: 0.66 ms in memory, 0.81 ms from the files, against 0.58 and 0.38 ms) | the dense top of the hierarchy (many arcs for each top node) | done: saved paths, memory layout, witness search, arc reduction; tileroute is now near OSRM CH and faster than OSRM MLD |
| More shortcuts than a graph partitioner would make | tile borders do not follow the roads | accepted for one tiling protocol |
| A query reads 8.8 MB of tile files (Kentucky) | the arcs of the low levels are large, and each arc is stored three times | a smaller arc, one arc list ([Tile files](../reference/tile-files.md#next-steps)) |
| Not a full product | no snapping of points to roads, no turn instructions, no server, no browser yet | later milestones |
