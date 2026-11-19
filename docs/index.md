# tileroute

tileroute finds the shortest path on a road network from duckOSM. It adds **shortcuts** to the graph and orders
them by **cells**: XYZ map tiles (the XYZ-tile cell version), or parts of the road graph with few roads across their borders
(the [partition version](explanation/partition.md)). The files can be the same XYZ tiles as duckOSM's builds, pbfcut
and the map tiles. The goal: a query reads only the files that it needs, so that a country does not need all of its
graph in memory, also in the browser.

!!! note "Status, 2026-11-19"
    The shortcuts and the query work in memory and from [tile files](reference/tile-files.md) on disk. Each route
    is equal to the route of duckOSM's router (Monaco: all 7.6 million pairs; Kentucky and four Vancouver areas:
    10,000 random pairs each). The [partition version](explanation/partition.md) (`--partition 16`) answers a France
    query in 0.54 ms from the files on a Mac mini (the XYZ-tile cell version: 5.32 ms). The browser version, the weights, the
    combined modes, the timetable and the distributed version are designed ([Plan](plan.md)), not made yet.

## Start

| To know this | Read |
|---|---|
| What a node and an arc are in this graph | [The graph](explanation/graph.md) |
| How a road edge gets a tile and a level | [Tiles and levels](explanation/levels.md) |
| How the cells come from the road graph, and why this is faster | [Cells from the graph (partition)](explanation/partition.md) |
| What a shortcut is, and how tileroute makes it | [Shortcuts](explanation/shortcuts.md) |
| How a query uses the shortcuts | [The shortest path](explanation/query.md) |
| Why and how the hierarchy is stored in tile files | [Tile files](explanation/tile-files.md) |
| What the top-down phase gives | [The top-down phase](explanation/top-down.md) |
| How to update the shortcuts after new costs | [Updates](explanation/updates.md) |
| How new costs are added fast, with one structure for many costs | [A structure with no costs](explanation/customization.md) |
| How a large area is prepared with little memory | [The preparation tile by tile](explanation/build-by-tile.md) |
| How the whole world can run on many servers | [The whole world (distributed)](explanation/distributed.md) |
| How a query uses its own weights for many items | [Weights](explanation/weights.md) |
| How driving, walking and cycling combine | [Modes](explanation/modes.md) |
| How trains and buses join the walking routes | [Timetable](explanation/timetable.md) |
| How driving uses predicted traffic for each time | [Predicted traffic](explanation/predicted-traffic.md) |
| What comes next, and in which order | [Plan](plan.md) |
| How tileroute is different from h3-routing-platform | [Comparison with h3-routing-platform](explanation/comparison.md) |
| How tileroute is different from OSRM, GraphHopper, Valhalla | [Comparison with other routers](explanation/engines.md) |
| How to run it | [The command line](reference/cli.md) |
| Time, memory and checks | [Measurements](reference/measurements.md) |
| How the hierarchy is stored in tile files, and what a query reads | [Tile files](reference/tile-files.md) |

## In one table

| Step | What happens |
|---|---|
| Input | duckOSM's `<mode>.edge_graph` and `<mode>.edges` |
| Level | Each road edge gets a level: the level of the smallest cell (an XYZ tile, or a partition cell) that holds both of its ends |
| Shortcuts | From the finest level to level 1, cell by cell: remove the road edges of that level, and join their neighbours with shortcuts |
| Query | Search from the start and from the target, only toward road edges of the same or a coarser level, until the two searches meet |
| Output | The path as duckOSM `edge_id`s, the same as duckOSM's router |

Sister project: [pbfcut](https://khoshkhah.github.io/pbfcut/) cuts `.osm.pbf` files into the same tiles.
