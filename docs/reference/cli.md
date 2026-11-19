# The command line

## Install

```bash
cargo install --path .
```

## tileroute check

Make the shortcuts of one mode of a duckOSM database, then compare routes with duckOSM's router.

```bash
tileroute check monaco.duckdb
```

```bash
tileroute check kentucky.duckdb --mode driving --pairs 10000
```

| Option | Default | Meaning |
|---|---|---|
| `--mode` | `driving` | The duckOSM mode: `driving`, `walking` or `cycling` |
| `--finest` | `20` | The finest level. A road edge with a higher `lca_zoom` gets this level. |
| `--pairs N` | all pairs | Compare N random pairs (a fixed sequence). `0`: make the shortcuts only. |
| `--lca` | off | Also run the top-down phase, and answer with the pruned query ([The top-down phase](../explanation/top-down.md)) |
| `--cch` | off | Use the triangle version (a customizable contraction hierarchy) ([Updates](../explanation/updates.md)) |
| `--changes K,K,...` | none | With `--cch`: change K random road edges, update, and check the costs and 1,000 routes on the changed graph |
| `--no-reference` | off | Do not compare with duckOSM's router; print only tileroute's time per query |
| `--write-pairs FILE` | none | Write the pairs and tileroute's costs as CSV (`source,target,cost`), for other routers |

The command reads the tile columns of duckOSM (`edges.from_tile`, `edges.to_tile`) when they are in the database. If
not, it gets the tiles from the node points (it needs DuckDB's `spatial` extension).

It prints the number of nodes and arcs, the shortcuts and the step times for each zoom, the tiles per query, the
result of the comparison, and the time for one query. It stops with exit code 2 if a route or an update is wrong.
