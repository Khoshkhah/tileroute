# A structure with no costs (customization)

!!! note "Status"
    Measured on Monaco and Kentucky, in memory (`tileroute check --partition 16 --cch`) and from the tile files
    (`tileroute build --cch`, `tileroute route --metric`). Not yet the default. The code is in
    [src/cch.rs](https://github.com/Khoshkhah/tileroute/blob/main/src/cch.rs) and
    [src/cchdisk.rs](https://github.com/Khoshkhah/tileroute/blob/main/src/cchdisk.rs).

## The problem

The hierarchy of [Shortcuts](shortcuts.md) uses the costs when it is made: the witness search keeps a shortcut only
when no other path is as cheap. So the structure is correct only for the costs that made it, and new costs run all
steps again. That is a problem for each case where the costs change often or differ between queries:

| Case | With costs in the structure |
|---|---|
| Traffic now (new costs every few minutes) | all steps again |
| Travel-time tables for roads ([Predicted traffic](predicted-traffic.md)): a cost for each hour | one structure for each hour |
| Weights for each query ([Weights](weights.md)) | a structure for each set of weights, or the hull corners of option B |
| Many metrics at the same time (car, truck, eco) | one structure for each metric |

## The idea: the structure first, the costs later

A customizable contraction hierarchy (CCH) splits the work in two parts. tileroute now has the same split:

| Part | Uses | Made | Kentucky |
|---|---|---|---|
| **1. Order** | the cells: the finest level first; in a level by cell; in a cell by node number | when the network changes | — |
| **2. Structure** | the graph and the order, **no costs** | when the network changes | 0.7 s, 18.5 million edges |
| **3. Customization** | the costs of one metric | for each new set of costs | 1.5 s (one thread) + 0.8 s arc reduction |
| **4. Query** | the structure and the costs of one metric | for each route | 0.38 ms |

## One network for all costs

The structure (which nodes, which edges) is made once and **all costs share it**. A type of cost (time, length, a
truck profile, the travel time of one hour) adds only numbers on the same edges. 5 types of cost are 1 structure and 5
sets of numbers, not 5 networks.

| Part | Shared by all costs | For each edge | Kentucky |
|---|---|---|---|
| Structure: the higher neighbours of each node | yes | one node number | 100 MB |
| Costs | no, one set for each cost | the cost a → b and the cost b → a | 296 MB (f64), 148 MB with f32 |
| Middle node (for the unpack) | no: the best path changes with the cost | one node for each direction | 148 MB; optional (the unpack can find it from the triangles) |
| Mask of the arc reduction | no | one bit for each direction: does the search use this edge for this cost? | 4.6 MB |

The arc reduction does not make a new network: it only sets the bits of the mask. The query reads the shared
structure and skips each edge whose bit is 0.

**1. Order.** Each node gets its own rank. With the [partition cells](partition.md) this is a nested-dissection order:
the nodes of a cut get a rank above all nodes of the two parts that the cut separates. This is the difference from the
sets of the other hierarchy, where all nodes of a set have the same rank.

**2. Structure.** The nodes are removed one at a time by rank. All higher neighbours of a removed node are joined,
with no witness search, because without costs nothing can show that an edge is not needed. The result is a graph
where the higher neighbours of each node are all joined to each other (a chordal graph). The code uses the
elimination tree: the lowest higher neighbour of a node gets its other higher neighbours. This takes one pass over
the nodes.

**3. Customization.** Each edge {a, b} (a lower) has two costs: a → b and b → a. They start with the costs of the
input arcs (infinite where there is no arc). Then, for each node x by rank, and for each pair y, z of its higher
neighbours:

```text
cost(y → z) = min(cost(y → z), cost(y → x) + cost(x → z))
cost(z → y) = min(cost(z → y), cost(z → x) + cost(x → y))
```

The edge keeps the x of its best cost. A shortest path between two nodes of higher rank than x that goes through x
is found this way, because the costs of the edges to x are final before x is used. After this, the
[arc reduction](shortcuts.md#arc-reduction-fewer-arcs-for-the-search) removes the search arcs that these costs do not
need (a "perfect customization" removes all of them; the arc reduction removes most).

**4. Query.** The same bidirectional search up the ranks as in a contraction hierarchy. The unpack uses the x of each
edge: the arc y → z with x is y → x, then x → z.

## Results: Kentucky

WSL PC, the same 2,000 pairs, one thread, in memory, `check --partition 16 --cch`:

| | Hierarchy with the witness search (today) | Structure with no costs, then customization |
|---|---|---|
| Made once for the network | — | order and structure: 0.7 s |
| For each new set of costs | all steps: 4.1 s | **customization: 2.3 s** (triangles 1.5 s on one thread, arc reduction and mask 0.8 s) |
| Edges | 7.2 million arcs | 18.5 million edges (each with two costs) |
| Search arcs | — | 18.7 million, 16.7 million after the arc reduction (as bits of a mask) |
| Memory for each cost | — (one hierarchy for each cost: 447 MB of query data) | 448 MB (f64 costs, middle nodes, mask); smaller forms in the table above |
| Nodes settled by one query | 680 | 572 |
| Arcs opened by one query | 8,789 | 15,644 (30,425 with no arc reduction) |
| One query | **0.21 ms** | 0.38 ms with the mask (0.46 ms with no arc reduction) |
| Routes | — | 2,000 of 2,000 the same as the hierarchy (Monaco: 20,000 of 20,000) |

**What this shows:**

- **New costs are faster** (2.3 s against 4.1 s), and the triangles still run on one thread. In parallel (the nodes
  of one depth of the elimination tree at the same time), the customization can take less than 1 s.
- **The structure does not depend on the costs**, so travel-time tables, weights for each query and many metrics can
  use one structure: each needs only its own costs (two numbers for each edge).
- **The price:** 2.6 times more edges, and queries slower (0.38 against 0.21 ms), because there is no witness
  search and the query reads the shared structure with a mask. A separate copy of the needed arcs for each cost
  gave 0.26 ms, but it costs 268 MB for each cost on Kentucky; the mask costs 4.6 MB.

## In the tile files

One folder holds the shared structure and one sub-folder for each metric:

```text
folder/
  index.bin, edges.idx    the files and the node of each duckOSM edge id (as for the hierarchy)
  structure/8_x_y.trt     for each node: its rank, its edge id, its higher neighbours (shared by all metrics)
  time/8_x_y.trt          for each edge of structure/8_x_y.trt, in the same order: two costs, two middle nodes, two mask bits
  length/8_x_y.trt        the same for the metric "length"
```

The files are the zoom-8 XYZ tiles (`--xyz-files`) or the partition cells, and the top file, as for the hierarchy. A
new set of costs writes only its own folder; the structure does not change.

```bash
tileroute build area.duckdb -o folder --partition 16 --xyz-files --cch --metrics time,length
tileroute route folder --metric time --pairs-file pairs.csv
```

**Kentucky** (WSL PC, the same 10,000 pairs, one thread):

| | Value |
|---|---|
| Build (in memory): read, structure 0.8 s, two customizations 2.4 s and 2.3 s, files 1.9 s | 17.7 s, peak 2.5 GB |
| `structure/` | 104 MB (shared) |
| `time/`, `length/` | 428 MB each |
| One query from the files, metric time | 0.39 ms, 4.5 files read |
| One query from the files, metric length | 0.54 ms, 4.5 files read |
| Memory of the query process | 590 MB (pages of the mapped files) |
| Routes from the files = routes in memory (time) | 2,000 of 2,000 |

The queries of the metric length are slower: with length as the cost, the search reaches more nodes before it can
stop. Each metric folder is 4 times the size of the structure: two costs (f64) and two middle nodes for each edge. A
smaller form is a next step: f32 costs where the sums stay exact, or no middle nodes (the unpack then finds the middle
node from the triangles).

## Weights for each query: the best of a few routes

A query can give its own weights: cost = w₁ · time + w₂ · length + … A customization for each query takes seconds,
so it is too slow. The cheap estimate: customize one metric for each cost type (and some fixed mixes) on the shared
structure; for a query, find the best route of each metric (the searches run in parallel), and keep the route with
the smallest cost for the weights of the query.

**The measurement** (`tileroute check --partition 16 --weights`): two cost types, time and length (the length of an
arc is the length of its first road edge, as duckOSM's cost is the time of its first road edge). A mix l gives
cost = l · time / (mean time) + (1 − l) · length / (mean length). The exact route of each mix comes from a
customization of that mix. The error is (cost of the estimate − cost of the exact route) / cost of the exact route.

**Kentucky, 2,000 random pairs:**

| l | Time and length routes: exact | mean error | largest error | Time, length and 50/50 routes: exact | mean error | largest error |
|---|---|---|---|---|---|---|
| 0.1 | 13.9% | 0.15% | 2.1% | 16.9% | 0.12% | 1.8% |
| 0.2 | 8.8% | 0.53% | 5.6% | 16.6% | 0.30% | 2.8% |
| 0.3 | 7.4% | 0.98% | 7.3% | 27.0% | 0.28% | 3.8% |
| 0.4 | 8.7% | 1.29% | 13.1% | 48.4% | 0.10% | 2.1% |
| 0.5 | 11.2% | 1.39% | 14.5% | 100% | 0% | 0% |
| 0.6 | 16.3% | 1.29% | 19.3% | 56.5% | 0.05% | 1.5% |
| 0.7 | 24.5% | 0.98% | 15.0% | 44.8% | 0.14% | 3.5% |
| 0.8 | 34.4% | 0.57% | 9.5% | 46.3% | 0.18% | 8.4% |
| 0.9 | 62.6% | 0.18% | 4.2% | 66.6% | 0.10% | 3.9% |

Monaco (5,000 pairs, short routes): with time and length, 96 to 98% exact, mean error below 0.08%; with the 50/50
route too, 98 to 100% exact.

**What this shows:**

- On long routes the estimate is seldom the exact route, but it is near: with time and length only, the mean error is
  at most 1.4%; one more candidate (the 50/50 mix) brings the mean error to 0.3% or less.
- The largest error of one pair can be large: up to 19% with two candidates, up to 8.4% with three.
- Each candidate costs one customization (2.3 s on Kentucky) and one metric in memory. The searches run in parallel,
  so the wait of one query is about one search; but each query uses one core for each candidate, so a busy server
  answers fewer queries per second.
- The exact method for any weights is [option B](weights.md) (the hull corners of each edge): no error, but more
  numbers for each edge. Not measured yet.

## Earlier: the triangle version with XYZ-tile cells

[Updates](updates.md) measured a triangle version before: with the XYZ-tile cells it had 4 times the edges, and 100
or more changes were slower than a rebuild. The partition order changes this: the cuts are small, so there are
fewer edges (2.6 times), and the full customization is faster than the steps.

## Next steps

| Step | Done means |
|---|---|
| The customization in parallel, by depth of the elimination tree | Kentucky: the customization below 1 s |
| A faster query on the shared structure: the costs of one direction next to the node numbers, f32 costs | Kentucky: near 0.26 ms with the mask |
| A smaller metric folder (f32 costs, no middle nodes) | Kentucky: a metric folder below 200 MB |
| The build of the structure and the customization one cell at a time, from disk | France on the Mac mini |
| Travel-time tables: one cost file for each hour on one structure | [Predicted traffic](predicted-traffic.md) with one structure |
| France on the Mac mini | customization time, edges, query time next to the hierarchy |
