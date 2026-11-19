# The graph

tileroute uses the same graph as duckOSM's router. It is **edge-based**: the line graph of the road network.

| Graph term | What it is | duckOSM | Name in h3-routing-platform |
|---|---|---|---|
| Node | One road edge in one direction | a row of `<mode>.edges` (`edge_id`) | "Edges" |
| Arc | A legal move from one road edge into the next one at their common junction | a row of `<mode>.edge_graph` (`from_edge`, `to_edge`, `cost`) | "Initial Shortcuts" |
| Cost of an arc | The travel time of its first road edge | `edge_graph.cost` (FLOAT) | `cost` |
| Shortcut | An added arc whose cost is the cost of a shortest path between its two nodes | made by tileroute | "Shortcuts" |

The junctions of the road network (duckOSM `<mode>.nodes`) are **not** nodes of this graph. A turn rule (for example
"no left turn") is a missing arc.

## Example: Burnaby, driving

| | Count |
|---|---|
| Junctions (not in the graph) | 17,303 |
| Nodes (road edges) | 40,825 |
| Arcs (`edge_graph` rows) | 111,513 |

## Which arcs

tileroute reads the same rows as duckOSM's router (`routed_graph`): for driving, the rows with `uses = 'car'`. If two
rows have the same `from_edge` and `to_edge`, the last one is used, as in networkx.

## The cost of a path

The cost of a path is the sum of the costs of its arcs, in f64, from left to right. Each arc cost is an f32 from the
database. A sum of f32 values in f64 is exact for road costs, so the order of the additions has no effect, and a
shortcut cost is exactly the sum that duckOSM's router makes.
