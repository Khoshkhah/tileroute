# Comparison with other routers

!!! warning "Not measured on the same computer yet"
    The numbers of the other routers on this page come from their papers and documentation, on other areas and other
    computers. They show only the order of size. A measurement of all routers on the same computer, with the same area
    and the same pairs, is milestone M2b of the [Plan](../plan.md).

## Road routers

| Router | Method | Preparation | One query | New costs (traffic) | Per-query weights | Reads by tile |
|---|---|---|---|---|---|---|
| OSRM (CH) | Contraction Hierarchies, nodes ordered by their importance, with a witness search | hours for Europe | below 1 ms (search), some ms with the path | a full preparation again | no | no: all in memory (or one memory-mapped file) |
| OSRM (MLD) | Customizable Route Planning: cells from a graph partitioner, cost tables between border nodes | partition one time; customization seconds to minutes | some ms | customization only | no (one profile for each build) | no |
| Bing Maps | Customizable Route Planning | as MLD | ms | customization: seconds | profiles | no |
| GraphHopper | Contraction Hierarchies (fixed), or Landmarks (A* with distances to some nodes, made one time) | CH as OSRM; Landmarks faster | CH: ms; Landmarks: tens of ms | Landmarks keep working with new costs | yes ("custom models", with Landmarks) | no |
| Valhalla | Tiles with three road-class levels, A* | the cut into tiles | tens to hundreds of ms for long routes | yes, traffic for each tile | yes (costing options) | yes, on demand, with a cache |
| Google Maps | not public | | | live traffic | | |
| **tileroute** | Shortcuts by XYZ tile (a contraction hierarchy ordered by tiles), exact | Kentucky 15 s | Kentucky 10.5 ms (one thread, in memory) | a rebuild of all costs (seconds) | planned, exact (hull corners, M5) | planned (M3) |

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
| Queries slower than OSRM CH (about 10 ms against below 1 ms) | no witness search (more shortcuts), hash maps in the search, each shortcut unpacked with a Dijkstra | flat arrays for the labels, a stored middle node for each shortcut |
| More shortcuts than a graph partitioner would make | tile borders do not follow the roads | accepted for one tiling protocol |
| Everything in memory today | the tile files are not made yet | M3 |
| Not a full product | no snapping of points to roads, no turn instructions, no server, no browser yet | later milestones |

## How the measurement on the same computer will work (M2b)

| Part | Plan |
|---|---|
| Routers | OSRM (CH and MLD), GraphHopper (CH and Landmarks), Valhalla, tileroute |
| Area | Kentucky (and Metro Vancouver), from the same `.osm.pbf` |
| Pairs | 10,000 random pairs of points, the same for all routers |
| Measures | preparation time and memory, disk size, time for one query (one thread), memory at query time |
| Route check | each router has its own road graph from OSM, so the routes cannot be equal edge by edge. The check compares the travel time of each route with duckOSM's router; a difference shows a different graph or a different speed model, not always an error. |
| Computer | one router at a time, on one computer |
