# Updates: new costs on the same network

Traffic changes the travel time of road edges, but the network stays the same. This page says how tileroute updates
its shortcuts after such a change, and how fast.

## Why the bottom-up hierarchy can be updated

The order of the nodes comes from the tiles, not from the costs. So the nodes, the sets and the places of the
shortcuts never change; only the costs of the shortcuts change. A classic contraction hierarchy (OSRM, GraphHopper)
orders the nodes by their costs, so a new cost needs a new order and a full preparation.

A change in tile T can change only the shortcuts of T and of the **ancestors** of T. Tiles that do not contain T keep
their shortcuts.

## Version 1: do the steps of the sets again

| Step | What happens |
|---|---|
| 1 | The changed arcs make their sets dirty (the set of the finer end of each arc). |
| 2 | From zoom 20 to zoom 1: each dirty set runs its step again (a Dijkstra from each in-node through the set). |
| 3 | If a shortcut cost changes, the set that uses that shortcut (in an ancestor tile) becomes dirty. If no cost changes, the update stops there. |

The problem: most of the time is in a few large steps at the coarse zooms. Metro Vancouver, one thread:

| Zoom | Sets | All steps | Largest step |
|---|---|---|---|
| 9 | 2 | 484 ms | 415 ms |
| 10 | 6 | 1,252 ms | 829 ms |
| 11 | 15 | 1,223 ms | 559 ms |
| 12 | 50 | 945 ms | 319 ms |
| 13 | 158 | 872 ms | 62 ms |
| 14 to 20 | 84,306 | 1,926 ms | 16 ms |

A change that reaches zoom 9 to 12 runs these large steps again: up to about 2.2 s, against 3.6 s for a full
preparation. So version 1 is fast only when a change stops at the fine zooms.

## Version 2: triangles (not kept)

The code was removed (milestone M2); it is in the git history, at commit `e39a78e` (`tileroute check --cch`).

The nodes are removed **one at a time**, in the same order as the sets (finest level first). This is a customizable
contraction hierarchy (CCH) whose order comes from the tiles.

| Part | What it is |
|---|---|
| Structure | When a node x is removed, each two of its neighbours that are still there get an edge. An edge {p, q} has two costs: p → q and q → p. The structure depends only on the network. |
| Triangle | For the edge {p, q}: a node x removed before p and q, with edges to both. |
| Cost of p → q | The smaller of its own arc (infinity if none) and, for each triangle x: cost(p → x) + cost(x → q) |
| Full customization | All edges, in the order of removal |
| Update | Only the edges that can change, in the order of removal (see below) |
| Query | The search of a contraction hierarchy; a shortcut is unpacked through the triangle that gives its cost |

**The update.** The changed arcs go into a queue, by the rank of their lower node. When an edge is taken from the queue,
its triangles are already final. If its cost changed, each edge above it (where it is a side of a triangle) gets one
test. That edge is computed again only if:

- the new triangle is cheaper than its cost, or
- the old triangle gave its cost, and the new triangle is more expensive.

All other edges keep their costs with no work.

**The checks.** After each update, the costs are equal to the costs of a full customization, and 1,000 routes on the
changed graph are equal to duckOSM's router on the changed graph (0 wrong).

## Results of version 2

Changes: random road edges, half of them × 1.5, the other half × 0.6.

| Area | Edges (shortcuts only) | Full customization | 1 change | 10 changes | 100 changes | 1,000 changes |
|---|---|---|---|---|---|---|
| Monaco | 19,994 (15,860) | 1.8 ms | 0.014 ms | 0.23 ms | 3.5 ms | 7.0 ms |
| Burnaby | 967,716 (875,461) | 0.47 s | 0.29 ms | 120 ms | 335 ms | 1.97 s |
| Vancouver city | 3,283,616 (3,091,832) | 4.6 s | 0.10 ms | 568 ms | 3.45 s | 23.2 s |
| Metro Vancouver | 11,977,843 (11,124,484) | 21 s | 0.8 ms | 120 ms | see Measurements | see Measurements |

## The decision: rebuild all

| Case | Version 2 (triangles) | Rebuild all (set version, 20 threads) |
|---|---|---|
| 1 to 10 changed road edges | below 1 ms to 0.6 s | 0.2 s (Burnaby) to 14 s (Kentucky) |
| 100 or more (for example one hour of traffic) | slower than a rebuild | 3.6 s (Metro Vancouver) |
| Memory | about 4 times the edges of the set version | as today |

Real traffic changes many road edges at the same time. The set version is fast because its steps run on all threads,
and it needs no extra structure. So **tileroute rebuilds all shortcut costs** when the costs change:

- The structure (tiles, levels, sets, shortcut places) stays; only the costs (or the vectors of
  [Weights](weights.md)) are made again.
- The rebuild runs on a schedule, for example every 15 minutes or every hour, and writes new tile files.
- For a country, the rebuild time must be measured (France). If it is too long, a rebuild by region (one zoom-8 tile
  and its ancestors) is the next step.

Version 2 and the top-down phase stay recorded on these pages. Their code was removed (milestone M2) and is in the git
history, at commit `e39a78e`.

For the rebuild, tileroute keeps **every shortcut pair** that a step makes, also when a direct arc is cheaper today
(`Hierarchy::pairs`). The pairs depend only on the network, so a rebuild with new costs uses the same list.
