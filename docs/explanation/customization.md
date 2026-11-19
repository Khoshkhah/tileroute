# A structure with no costs (customization)

!!! note "Status"
    Measured in memory on Monaco and Kentucky: `tileroute check --partition 16 --cch`. Not yet in the tile files and
    not yet the default. The code is in [src/cch.rs](https://github.com/Khoshkhah/tileroute/blob/main/src/cch.rs).

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
| **3. Customization** | the costs of one metric | for each new set of costs | 1.5 s (one thread) + 0.7 s arc reduction |
| **4. Query** | the structure and the costs of one metric | for each route | 0.26 ms |

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
| For each new set of costs | all steps: 4.1 s | **customization: 2.2 s** (triangles 1.5 s on one thread, arc reduction 0.7 s) |
| Edges | 7.2 million arcs | 18.5 million edges (each with two costs) |
| Search arcs | — | 18.7 million, 16.7 million after the arc reduction |
| Nodes settled by one query | 680 | 572 |
| Arcs opened by one query | 8,789 | 15,644 (30,425 with no arc reduction) |
| One query | **0.21 ms** | 0.26 ms (0.38 ms with no arc reduction) |
| Routes | — | 2,000 of 2,000 the same as the hierarchy (Monaco: 20,000 of 20,000) |

**What this shows:**

- **New costs are faster** (2.2 s against 4.1 s), and the triangles still run on one thread. In parallel (the nodes
  of one depth of the elimination tree at the same time), the customization can take less than 1 s.
- **The structure does not depend on the costs**, so travel-time tables, weights for each query and many metrics can
  use one structure: each needs only its own costs (two numbers for each edge).
- **The price:** 2.6 times more edges, and queries about 25% slower (0.26 against 0.21 ms), because there is no
  witness search. The arc reduction gives back most of the query time.

## Earlier: the triangle version with XYZ-tile cells

[Updates](updates.md) measured a triangle version before: with the XYZ-tile cells it had 4 times the edges, and 100
or more changes were slower than a rebuild. The partition order changes this: the cuts are small, so there are
fewer edges (2.6 times), and the full customization is faster than the steps.

## Next steps

| Step | Done means |
|---|---|
| The customization in parallel, by depth of the elimination tree | Kentucky: the customization below 1 s |
| The structure and the costs in the tile files (the costs as a separate file of numbers for each metric) | a new set of costs writes only the cost file; the routes from the files are equal to memory |
| Travel-time tables: one cost file for each hour on one structure | [Predicted traffic](predicted-traffic.md) with one structure |
| France on the Mac mini | customization time, edges, query time next to the hierarchy |
