# Measurements

All numbers are for **driving**, on a WSL PC with 20 cores and 23 GB, with tileroute at commit `0e40180` (2026-11-18).
"Time" is the time to read the database and make the shortcuts. "Memory" is the peak RSS of the process.

## Making the shortcuts

| Area | Nodes | Arcs | Shortcuts | Time | Memory |
|---|---|---|---|---|---|
| Monaco | 2,765 | 4,937 | 4,076 | 0.09 s | — |
| Somerset | 7,024 | 19,618 | 36,760 | 0.09 s | 59 MB |
| Burnaby | 40,824 | 111,513 | 241,245 | 0.25 s | 124 MB |
| Vancouver city | 75,779 | 225,932 | 953,742 | 1.2 s | 282 MB |
| Metro Vancouver | 374,907 | 1,028,495 | 3,026,705 | 3.9 s | 917 MB |
| Kentucky | 2,149,438 | 5,645,321 | 10,318,657 | 17.4 s (read 3.6 s, shortcuts 13.8 s) | 3.8 GB ¹ |

¹ The peak of a full check run: it includes duckOSM's reference graph and 10,000 routes.

## Routes

Each check compares tileroute with duckOSM's router (`duckosm-router`, networkx order), on all threads.

| Area | Pairs | Same path | Tie | No path | Wrong | tileroute | duckOSM router |
|---|---|---|---|---|---|---|---|
| Monaco | 7,645,225 (all) | 7,223,187 | 0 | 422,038 | 0 | 17.9 s | 18.6 s |
| Somerset | 10,000 | 9,747 | 77 | 176 | 0 | 0.15 s | 0.10 s |
| Burnaby | 10,000 | 9,870 | 12 | 118 | 0 | 0.47 s | 0.91 s |
| Vancouver city | 10,000 | 9,934 | 13 | 53 | 0 | 2.06 s | 3.10 s |
| Metro Vancouver | 10,000 | 9,880 | 50 | 70 | 0 | 4.57 s | 41.25 s |
| Kentucky | 10,000 | 9,664 | 0 | 336 | 0 | 10.8 s | 322.8 s |

On a small area the hierarchy gives no gain. On Kentucky, one route takes about 21 ms with tileroute and 645 ms with
duckOSM's router. These routes run in memory; the tile files on disk are not made yet.
