# Measurements

All numbers: **driving**, on a WSL PC with 20 cores and 23 GB, 2026-11-18, one run at a time. Kentucky is a duckOSM
build of 2026-11-18; Somerset, Burnaby, Vancouver city and Metro Vancouver are the databases of h3-routing-platform.

## Making the shortcuts (bottom-up, 20 threads)

| Area | Nodes | Arcs | Shortcuts | Read | Shortcut steps | Peak memory ¹ |
|---|---|---|---|---|---|---|
| Monaco | 2,765 | 4,937 | 4,076 | 0.07 s | 0.007 s | — |
| Somerset | 7,024 | 19,618 | 36,760 | 0.08 s | 0.04 s | 63 MB |
| Burnaby | 40,824 | 111,513 | 241,245 | 0.1 s | 0.2 s | 123 MB |
| Vancouver city | 75,779 | 225,932 | 953,742 | 0.2 s | 1.5 s | 274 MB |
| Metro Vancouver | 374,907 | 1,028,495 | 3,026,705 | 0.4 s | 3.6 s | 873 MB |
| Kentucky | 2,149,438 | 5,645,321 | 10,318,657 | 3.7 s | 15.3 s | 3.5 GB |

¹ The peak of a whole run: read, shortcuts and 10,000 queries (Kentucky: 2,000).

## Routes: correctness

Each route is compared with duckOSM's router (`duckosm-router`, networkx order).

| Area | Pairs | Same path | Tie | No path | Wrong |
|---|---|---|---|---|---|
| Monaco | 7,645,225 (all) | 7,223,187 | 0 | 422,038 | 0 |
| Somerset | 10,000 | 9,747 | 77 | 176 | 0 |
| Burnaby | 10,000 | 9,870 | 12 | 118 | 0 |
| Vancouver city | 10,000 | 9,934 | 13 | 53 | 0 |
| Metro Vancouver | 10,000 | 9,880 | 50 | 70 | 0 |
| Kentucky | 2,000 | 1,924 | 0 | 76 | 0 |

A tie is another path with exactly the same cost ([The shortest path](../explanation/query.md#equal-to-duckosms-router)).

## Routes: time for one query (one thread)

| Area | tileroute | duckOSM router | tiles touched |
|---|---|---|---|
| Somerset | 0.14 ms | 0.18 ms | 11.6 |
| Burnaby | 0.63 ms | 1.9 ms | 13.1 |
| Vancouver city | 2.5 ms | 7 to 11 ms | 14.5 |
| Metro Vancouver | 5.2 ms | 85 to 89 ms | 16.9 |
| Kentucky | 10.5 ms | 700 to 810 ms | 20.6 |

The duckOSM router times come from runs with 20 threads, so they are a little higher than on one thread. These queries
run in memory; the tile files are not made yet ([Plan](../plan.md)).

## The time of each step, Metro Vancouver (one thread)

| Zoom | Sets | All steps | Largest step |
|---|---|---|---|
| 9 | 2 | 484 ms | 415 ms |
| 10 | 6 | 1,252 ms | 829 ms |
| 11 | 15 | 1,223 ms | 559 ms |
| 12 | 50 | 945 ms | 319 ms |
| 13 | 158 | 872 ms | 62 ms |
| 14 to 20 | 84,306 | 1,926 ms | 16 ms |

## Experiments not kept

| Experiment | Result | Page |
|---|---|---|
| Top-down phase, query pruned at the LCA | 7 to 20% faster queries, the same tiles touched, 3 to 10 times the preparation (Kentucky: +109 s) | [The top-down phase](../explanation/top-down.md) |
| Triangle version (CCH) with updates | 1 change below 1 ms, but 100 or more changes slower than a rebuild; 4 times the edges | [Updates](../explanation/updates.md) |

For h3-routing-platform on the same areas, see [Comparison](../explanation/comparison.md#measured-on-the-same-input).
