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

The command reads the tile columns of duckOSM (`edges.from_tile`, `edges.to_tile`) when they are in the database. If
not, it gets the tiles from the node points (it needs DuckDB's `spatial` extension).

It prints the number of nodes and arcs, the shortcuts for each zoom, the result of the comparison, and the time. It
stops with exit code 2 if a route is wrong.
