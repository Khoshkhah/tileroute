# Tile files

`tileroute build` writes the hierarchy of shortcuts to files on disk, grouped by XYZ tile. `tileroute route` answers
queries from these files only. A query maps a file only when its search gets to a node in that file. The approach
is in [Tile files (explanation)](../explanation/tile-files.md).

## The folder

| File | Holds |
|---|---|
| `top.trt` | The nodes of level 7 and lower (their cell is a tile of zoom 7 or coarser) |
| `8_<x>_<y>.trt` | The nodes whose cell is in this zoom-8 tile (levels 8 to 20) |
| `edges.idx` | The duckOSM `edge_id`s, sorted, and the node number of each |
| `index.bin` | For each `.trt` file: its first node number, its number of nodes, its tile key |

A node number is the same in all files. The nodes of one file are one range of numbers, so the file of a node is a
binary search in `index.bin`. In a file, the nodes are sorted by level, then by cell.

## One `.trt` file

A header (the magic `TRTILE01`, the first node, the number of nodes, and the offset and size of each section), then
the sections. All numbers are little-endian. Each section starts at a multiple of 8 bytes, so the reader uses the memory
map directly, with no decode step.

| Section | Type | For |
|---|---|---|
| Level of each node | u8 | the order of the hierarchy |
| Cell of each node | u64 (tile key) | statistics |
| `edge_id` of each node | i64 | the output |
| Forward-search arcs | offsets u32, then (cost f64, other node u32) | the search from the start |
| Backward-search arcs | the same | the search from the target |
| All arcs | offsets u32, other node u32 (sorted), (cost f64, path start u32, path length u32) | the unpack |
| Inner nodes of the shortcuts | u32 | the unpack |

The arcs of a node are in the file of that node. The inner nodes of a shortcut are in the file of its first node.

## The query

1. `edges.idx` gives the node numbers of the start edge and the target edge (a binary search in the memory map).
2. The bidirectional search is the same as in memory. When it settles a node, it maps the file of that node, if the
   query did not map it before. The labels are in hash maps, so the memory of a query is proportional to the nodes
   that it reaches, not to the size of the area.
3. The unpack reads the arcs of the path, and the `edge_id` of each node on it. This reads the files along the route.

A small cache keeps the last 64 open files (`--cache`). The operating system keeps the file pages in its own cache.

## Correctness

| Area | Pairs | Equal to the route in memory | Wrong against duckOSM's router |
|---|---|---|---|
| Monaco | 7,645,225 (all) | 7,645,225 | 0 |
| Kentucky | 2,000 (`data/bench/kentucky/points2000.csv`) | 2,000 | 0 |

"Equal" means the same cost and the same path, or no path in both. The check: `tileroute check <db> --files <folder>`.

## Measurements: Kentucky

WSL PC (20 cores, 23 GB), 2026-11-19, driving, the duckOSM build of 2026-11-18 (2,149,438 nodes, 15,288,968 arcs
with the shortcuts). Nothing else heavy ran.

### Files

| File | Number | Size |
|---|---|---|
| `top.trt` | 1 | 47.0 MB (3,340 nodes) |
| zoom-8 tile files | 18 | 720.1 MB in all; mean 40.0 MB; largest 245.9 MB (`8_67_98`, Louisville, 668,580 nodes); smallest 488 bytes |
| `edges.idx` | 1 | 25.8 MB |
| `index.bin` | 1 | 320 bytes |
| All | 21 | **792.9 MB** |

`tileroute build`: 22.6 s in all (read 4.2 s, shortcuts 17.5 s, write 0.8 s), peak memory 3.5 GB.

### Queries

`tileroute route <folder> --pairs-file points2000.csv`: one thread, a process that only maps the files.

| Number | Value |
|---|---|
| Time for one query, first pass | 2.66 ms |
| Time for one query, second pass | 2.61 ms |
| Time for one query in memory (`check`, one thread), for comparison | 2.36 ms |
| Tile files read by one query | mean 3.7, largest 8 (of 19) |
| Bytes read by one query | mean 8.8 MB, largest 17.5 MB |

The bytes read are the sections that the query reads: the arcs of the settled nodes, the arcs of the unpack and the
edge ids. Most of them are the arcs of the nodes of low level, which have many shortcuts. The query from the files is
about 10% slower than in memory: the labels are in hash maps, and each settled node needs a search for its file.

### Memory of the query-only process

| Queries | Files mapped | Peak memory (resident) | Of which: file pages | Of which: own memory |
|---|---|---|---|---|
| 0 (files opened) | 0 | 11.6 MB | | |
| 1 | 3 | 33.4 MB | 31.8 MB | 0.8 MB |
| 10 | 8 | 207.6 MB | 201.4 MB | 0.8 MB |
| 100 | 14 | 483.9 MB | 471.2 MB | 0.9 MB |
| 2,000 | 16 | **764.3 MB** | 744.5 MB | 1.4 MB |

The peak memory counts the pages of the mapped files that the process read. These pages are in the cache of the
operating system: the system can drop them when it needs memory, and other processes that map the same files share
them. The own memory of the process (labels, heaps, the index) stays below 2 MB. After 2,000 random pairs across the
state, the process read almost all of the files, so its resident memory is near the size of the files.

One query reads 3.5 MB of sections, but the resident memory increases by about 22 MB, because the operating system
reads more pages around each page that the query touches.

## Next steps

| Problem | Effect | Possible change |
|---|---|---|
| The largest file (Louisville) is 246 MB | a query in that city maps a large file | files of zoom 9 or 10 in dense areas |
| The arcs of low levels are most of the bytes read | 8.8 MB for each query | a smaller arc (f32 cost where it is exact, u16 local numbers) |
| Each arc is stored three times (forward, backward, all) | larger files | one arc list with indices |
| No archive of the folder yet | the browser cannot read it | `tileroute pack` (M8) |
