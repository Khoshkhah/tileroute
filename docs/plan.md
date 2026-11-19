# Plan

This page is the plan from today to a router with dynamic costs, per-query weights, many modes and timetables, by
tile, natively and in the browser.

## Where we are (2026-11-18)

| Part | Status |
|---|---|
| Shortcuts by tile (bottom-up, set version) | done, 0 wrong routes on all checks |
| Query in memory | done: Kentucky 24 ms against 811 ms for duckOSM's router |
| Top-down phase, triangle version | measured, not kept, code removed ([The top-down phase](explanation/top-down.md), [Updates](explanation/updates.md)) |
| Every shortcut pair kept (for the rebuild and the weights) | done (M2) |
| Tile files on disk, browser | not started |
| Weights, modes, timetable | design on these pages |

## The decisions

| Question | Decision | Why |
|---|---|---|
| The method | Shortcuts by tile, bottom-up only | exact, fast preparation, one structure for all costs |
| New costs (traffic) | Rebuild all costs on a schedule | faster than an update when many road edges change; no extra structure |
| Per-query weights | Option B: each shortcut keeps the corners of the hull of its vectors | exact for any weights ≥ 0; no preparation for each query |
| Combined modes | One graph with change arcs; a rule becomes a copy of the graph for each state | the hierarchy works unchanged |
| Timetable | RAPTOR (McRAPTOR for weights), joined to the walking hierarchy at the stops | a timetable does not fit fixed-cost shortcuts |
| Predicted traffic for driving | Time slices (one cost rebuild for each slice), then slices along the route for long trips | uses the structure and the rebuild that tileroute has |

## One model for all modes

All modes can use one model, as a timetable does:

| Part of the model | Driving | Walking, cycling | Bus, train |
|---|---|---|---|
| Time on an arc | a function of the entry time (predicted traffic) | a fixed time | the timetable (departure and arrival times) |
| Other items (turns, lights, transfers, walking) | fixed numbers on the arcs | fixed numbers | fixed numbers (transfers, waiting) |
| Cost of a trip | the query weights × the items, with the arrival time as the time item | the same | the same |

**The rule that this model adds.** When the time on an arc depends on the entry time and the other items have weights,
the best route to a node is not always the cheapest label: a label that is a little more expensive but arrives earlier
can catch a faster time (or a bus) later. So the search must keep, for each node, the labels that are not worse in both
**arrival time** and **other cost** (two criteria), as McRAPTOR does for transit. With weight 0 on all other items, one
label for each node is sufficient again.

**How the parts work together in this model.**

| Part | Role |
|---|---|
| The shortcut hierarchy (fixed costs, the smallest time over all slices) | a fast lower bound: it shows the corridor where the best route can be |
| The time-dependent search with two criteria | the exact answer, only in that corridor, on the base arcs, the predicted times and the timetable |
| RAPTOR rounds | the transit part of that search |

So the shortcuts do not hold time functions; they only make the exact time-dependent search small.

## The milestones

| Step | Work | Done means |
|---|---|---|
| **M2 Clean up** (done) | Remove the top-down and triangle code; keep every shortcut pair (needed for the vectors and for the rebuild) | all checks pass; the code has one method |
| **M2b Same-computer comparison** (done) | Install OSRM, GraphHopper and Valhalla; build Kentucky (and Metro Vancouver) from the same `.osm.pbf`; run the same 10,000 pairs on one thread, one router at a time ([Comparison with other routers](explanation/engines.md)) | one table: preparation, disk, memory, query time, and the travel-time difference to duckOSM's router for each router |
| **M2c Faster and smaller queries** (in progress: saved paths, flat arrays, stall-on-demand and the witness search are done; Kentucky 10.5 → 2.35 ms with the memory layout; the dense top remains) | Witness search, the middle node of each shortcut for the unpack, flat arrays and stall-on-demand in the search, one compact arc list ([why](explanation/engines.md#why-tileroute-is-slower-and-larger-today)) | the same routes (0 wrong); Kentucky query time and query memory measured again on the Mac mini, next to OSRM and GraphHopper |
| **M3 Tile files** | One file for each zoom-8 tile with the shortcuts of its tiles, an archive of all files, memory-mapped reads, the edge index | a query reads only its tiles; the routes are equal to the in-memory routes; reads and time measured for each query |
| **M4 Rebuild** | `tileroute rebuild` with new costs from duckOSM (or from SonoFlow flows), structure kept, new tile files | the routes equal a new preparation; rebuild time of Kentucky and France (Mac mini) measured |
| **M5 Items and weights** | duckOSM gives the items (time, length, turn types, traffic lights) as columns of `edge_graph`; tileroute keeps the hull corners for each shortcut; the query takes the weights | for random weights, 0 wrong routes against duckOSM's router with the same cost; corners, size and time measured for 2 and 3 items |
| **M6 Many modes** | Walking and cycling checks; one-to-many and many-to-many queries; combined graphs (free change, then rules), with the same model as duckOSM's `route_multimodal` | each mode and each combined graph: 0 wrong routes against duckOSM |
| **M7 Timetable** | GTFS input, stops with tile keys, walking transfers from the hierarchy, RAPTOR, then McRAPTOR with weights; door-to-door join | equal to a plain time-dependent search for random stops and times |
| **M7b Predicted traffic** | Slices from SonoFlow's prediction (flow to time, FIFO), one rebuild for each slice, the query by departure time; then the search that changes slices along the route | for each slice, 0 wrong routes against duckOSM's router with that slice; along the route, equal to a plain time-dependent Dijkstra |
| **M8 Browser** | WebAssembly package, archive read with HTTP range reads or from browser storage, a demo page | the browser gives the same routes as native, for driving, walking and a timetable trip |

M5, M6, M7 and M7b need M3 and M4 (the tile files), because their data is also stored by tile. M5 and M6 can run in parallel.

## What tileroute needs from duckOSM

| Data | For | Status |
|---|---|---|
| `nodes.tile`, `edges.from_tile`, `edges.to_tile`, `edges.lca_zoom` | all | being added (build option) |
| Turn type of each arc (left, right, U-turn, straight) | M5 | new column of `edge_graph` |
| Traffic light at the junction of each arc | M5 | new column (from `highway=traffic_signals`) |
| Change places (parking, bicycle parking, station entrances) | M6 | duckOSM `route_multimodal` model |
| GTFS stops linked to walking road edges | M7 | new |
| Predicted travel time for each road edge and slice | M7b | from SonoFlow, through duckOSM edge ids |

## Risks

| Risk | Effect | What we do |
|---|---|---|
| Many hull corners for 3 or more items | large tiles, slow preparation | measure in M5 with 2 items first; limit the number of items |
| Rebuild of a country is too slow | old costs for too long | rebuild by region (one zoom-8 tile and its ancestors) |
| Tile borders do not follow the roads | more shortcuts than a graph partitioner | accepted for one tiling protocol; measured in M3 |
| Ties | another path with the same cost than duckOSM | count them in each check; a tie-break item (for example the number of turns) removes most of them |
