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
| `--pairs-file CSV` | none | Use these pairs: a CSV with the duckOSM edge ids of the start and the target in its first two columns, and a header line |
| `--profile` | off | Print where the time of a query goes: nodes settled, search time, unpack time |
| `--limit K` | none | An experiment, not exact: the search may not go above the ancestor K zooms coarser than the common tile of the start and the target; prints how many routes become longer |
| `--core Z` | 0 | Stop the steps at zoom Z; the coarser nodes stay as one graph that the query searches in both directions (an experiment; slower on Kentucky) |
| `--partition LEAF` | off | The cells from the inertial flow partition of the road graph, not from the XYZ tiles; cut until a cell has at most LEAF junctions (for example 16) ([The whole world](../explanation/partition.md)) |
| `--no-reference` | off | Do not compare with duckOSM's router; print only tileroute's time per query |
| `--write-pairs FILE` | none | Write the pairs and tileroute's costs as CSV (`source,target,cost`), for other routers |
| `--files DIR` | none | Also answer each pair from the tile files in `DIR` (from `tileroute build`), and compare with the route in memory. Stops with exit code 2 if a route is wrong. |

The command reads the tile columns of duckOSM (`edges.from_tile`, `edges.to_tile`) when they are in the database. If
not, it gets the tiles from the node points (it needs DuckDB's `spatial` extension).

It prints the number of nodes and arcs, the shortcuts and the step times for each zoom, the tiles per query, the
result of the comparison, and the time for one query. It stops with exit code 2 if a route is wrong.

## tileroute build

Make the shortcuts of one mode, then write them to tile files in a folder ([Tile files](tile-files.md)).

```bash
tileroute build kentucky.duckdb -o kentucky.tiles
```

| Option | Default | Meaning |
|---|---|---|
| `-o DIR` | (necessary) | The folder for the files. The command makes it if it does not exist. |
| `--mode` | `driving` | The duckOSM mode |
| `--finest` | `20` | The finest level |
| `--by-tile` | off | Prepare one zoom-8 tile at a time, with much less memory ([The preparation tile by tile](../explanation/build-by-tile.md)); the files are the same format |
| `--xyz-files` | off | With `--partition` (in memory or `--by-tile`): one file for each zoom-8 XYZ tile, not for each partition cell ([The three versions](../explanation/partition.md#the-three-versions)) |
| `--partition LEAF` | off | With `--by-tile`: the cells from the partition, as for `check` |
| `--top Z` | 7 (2 with `--partition`) | With `--by-tile`: the nodes of level Z and coarser are the top; one file for each cell of zoom Z + 1 |

It prints the number of files, the size of the top file, of the zoom-8 tile files and of the edge index. With
`--by-tile` it prints the time of each phase and the peak memory of the process.

## tileroute slim

Copy only the tables that tileroute reads (the arcs of the mode, the road edges, the points of their ends) into a new,
small database. France: 28.5 GB → 2.0 GB.

```bash
tileroute slim france.duckdb france-slim.duckdb --mode driving
```

## tileroute route

Answer pairs from the tile files only. The process does not open the duckOSM database and does not make shortcuts, so
its memory is the memory of a query server.

```bash
tileroute route kentucky.tiles --pairs-file points2000.csv
```

| Option | Default | Meaning |
|---|---|---|
| `--pairs-file CSV` | (necessary) | The pairs: the duckOSM edge ids of the start and the target in the first two columns, and a header line |
| `--cache N` | `64` | The number of open (memory-mapped) tile files that the process keeps |

It answers all pairs two times on one thread. The first pass maps the files; in the second pass the files are in the
cache of the operating system. For each pass it prints the time for one query, and the tiles and bytes that one query
reads. At the end it prints the peak memory of the process.
