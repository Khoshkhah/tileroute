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
| **tileroute** | 0.1 | read 1.5 s + shortcuts 6.1 s | 5.2 GB ¹ | (no files yet) | 5.3 ms | 1,168 MB ² |

- The router queries go over HTTP on the same computer, one at a time with one kept connection: their time includes a
  small HTTP cost (about 0.1 ms). tileroute is called directly. Memory at query time: the server process after the
  10,000 queries.
- ¹ The peak of the whole run (read, shortcuts, queries).
- ² The data that a query needs: the hierarchy (1,117 MB) and the edge ids (52 MB). The process holds more, because the
  check also keeps duckOSM's reference graph.
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
| Nodes that the search settles | 4,226 (a good contraction hierarchy: some hundreds) |
| Search | 7.4 ms (WSL PC) |
| Unpack (shortcuts back to road edges) | 3.1 ms |
| Nodes on the path | 433 |

| Cause | Effect | Fix |
|---|---|---|
| The order comes from the tiles only, and there is no witness search | the top of the hierarchy is dense (zoom 6: 137,928 shortcuts for 360 nodes), so the search opens many arcs | a witness search (skip a shortcut when a path of the same cost exists without it); important roads higher inside each set |
| Each shortcut is unpacked with a Dijkstra in its tile | 3.1 ms for each query | store the middle node of each shortcut |
| Labels in hash maps; no stall-on-demand | slower steps | flat arrays; stall-on-demand |
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
| **tileroute** | Shortcuts by XYZ tile (a contraction hierarchy ordered by tiles), exact | Kentucky 7.6 s (Mac mini) | Kentucky 5.3 ms (Mac mini, one thread, in memory) | a rebuild of all costs (seconds) | planned, exact (hull corners, M5) | planned (M3) |

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
| Queries slower than OSRM CH (5.3 ms against 0.58 ms on the same Mac) | no witness search (more shortcuts), hash maps in the search, each shortcut unpacked with a Dijkstra | flat arrays for the labels, a stored middle node for each shortcut |
| More shortcuts than a graph partitioner would make | tile borders do not follow the roads | accepted for one tiling protocol |
| Everything in memory today | the tile files are not made yet | M3 |
| Not a full product | no snapping of points to roads, no turn instructions, no server, no browser yet | later milestones |
