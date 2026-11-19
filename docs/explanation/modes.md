# Modes: driving, walking, cycling, and combined

!!! note "Status: one mode works, combined modes are a plan"
    `tileroute check --mode walking` and `--mode cycling` read those duckOSM modes today. Combined modes are the design
    on this page.

## One mode

Each duckOSM mode has its own edge graph (`driving.edge_graph`, `walking.edge_graph`, `cycling.edge_graph`). tileroute
makes one hierarchy for each mode, with the same tiles and the same levels:

| Mode | Graph | Items for the cost ([Weights](weights.md)) |
|---|---|---|
| Driving | `driving.edge_graph` (`uses = 'car'`) | time, length, turns, traffic lights |
| Walking | `walking.edge_graph` | time, length, steps, slope, lights (crossings) |
| Cycling | `cycling.edge_graph` | time, length, slope, surface, road class |

## Combined modes

A combined trip uses more than one mode, for example "cycle to the station, then walk". There are two kinds:

| Kind | Example | Model | Hierarchy |
|---|---|---|---|
| **Free change** | walk or push the bike at any time | One graph with the nodes of both modes, and a "change" arc between the nodes of the two modes at the same junction | The same hierarchy works unchanged |
| **Change with a rule** | drive, park, then walk; never drive again after walking | One copy of the graph for each **state** (driving, walking), and one-way change arcs (driving → walking at a parking place) | The same hierarchy works unchanged on this larger graph |

The rule becomes part of the graph: there is no arc from the walking copy back to the driving copy, so no route can
drive again after it walks. This is the "product graph" of the graph and the rule.

## The change arcs

A change arc is an arc like any other, with its own items:

| Item | Example |
|---|---|
| Time | 2 minutes to park and lock the car |
| Mode changes | 1 (so that the query weights can make each change expensive) |
| Cost | a parking fee as an item, if wanted |

The change places come from duckOSM: parking places, bicycle parking, station entrances. duckOSM's `route_multimodal`
already has a model for this; tileroute must use the same model and the same rules, so that the routes are equal.

## Size

A combined graph has the nodes of all its modes (and of all its states). It is larger than one mode, but it uses the
same tiles: a tile of the combined graph holds the nodes of all modes in that tile. The preparation and the queries do
not change.

## Checks

The rule stays the same: each route must equal duckOSM's router (`route_multimodal`) on the same combined graph.
