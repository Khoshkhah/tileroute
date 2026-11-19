# The top-down phase

h3-routing-platform has two passes: bottom-up, then top-down. tileroute has the top-down phase as an option
(`tileroute check --lca`). This page says what it does and what the measurements show.

## What it does

| Part | What it is |
|---|---|
| Exits of a tile T (zoom z) | The nodes of a level lower than z with an arc to or from a node of level z or higher in T: the road edges that come into T or go out of T |
| Table of T | The **global** shortest cost between each two exits of T, also when the best path leaves T |
| Top-down pass | From zoom 1 to the finest level: the table of T comes from a small Dijkstra on the nodes of level zoom(T) − 1 in the parent P and the exits of P, with the table of P between the exits of P |
| Pruned query | The search stays in the LCA tile of s and t (only nodes of level L or higher, L = zoom of the LCA). When it reaches an exit, it stops there. The best path is the smaller of: the meeting inside, and (forward to exit y) + table(y, y′) + (backward from exit y′) |

The pruned query is exact: a path that leaves the LCA tile goes out at an exit and comes in at an exit, and the table
has the global cost between them. The checks show 0 wrong routes (Monaco: all 7.6 million pairs).

## What the measurements show

| Area | Bottom-up | Top-down (extra) | Table cells | Query: up / pruned | Tiles per query: up / pruned |
|---|---|---|---|---|---|
| Somerset | 0.04 s | 0.1 s | 0.3 million | 0.33 / 0.30 ms | 11.6 / 12.5 |
| Burnaby | 0.2 s | 0.9 s | 2.1 million | 1.3 / 1.0 ms | 13.1 / 13.8 |
| Vancouver city | 1.5 s | 9.3 s | 6.7 million | 5.6 / 4.7 ms | 14.5 / 15.2 |
| Metro Vancouver | 3.6 s | 30.3 s | 25.5 million | 9.8 / 9.1 ms | 16.9 / 17.4 |

(Query times from runs with 20 threads; see [Measurements](../reference/measurements.md) for the one-thread times.)

- The query is only 7 to 20% faster.
- The tiles per query do not go down. The search of the bottom-up hierarchy already stops by distance: it only reaches
  nodes nearer than the best path found so far, so it stays near the route. The pruned query also touches the table
  and the exits.
- The top-down pass takes 3 to 10 times the time of the bottom-up pass, and its tables are large.
- A change of one arc cost can change the tables of many tiles everywhere, so the tables do not support fast updates
  ([Updates](updates.md)).

## Where it can still help

With the tiles in files, the pruned query knows **before the search** which tiles it needs: the tiles in the LCA tile
and one table. In the browser, it can then read all of them at the same time. The bottom-up query finds its tiles
during the search, so it reads them one after another. This is not measured yet: the decision waits for the tile files.
