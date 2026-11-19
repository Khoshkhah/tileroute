# tileroute

Route on a road network that is stored by XYZ map tiles. A query reads from disk only the tiles that it needs, so a
country does not need all of its graph in memory. The same Rust code runs in the browser (WebAssembly).

Status: the shortcuts and the query work in memory and from tile files on disk, and each route equals duckOSM's router.
With the partition version (`--partition 16`), a France query takes 0.54 ms from the files on a Mac mini. The browser
version is not made yet.

Documentation: [docs/index.md](docs/index.md) (the site, built with `mkdocs build`).

| | |
|---|---|
| Input | A duckOSM database: `<mode>.edge_graph` and `<mode>.edges` |
| Method | Shortcuts by cell (a contraction hierarchy ordered by nested cells): XYZ tiles, or an inertial flow partition of the road graph |
| Routes | Equal to duckOSM's router: the same `edge_id`s and the same time |
| Files | One file for each zoom-8 XYZ tile (or each partition cell) and one top file |
| Runs on | Command line, Rust, Python (calls Rust), browser |

Sister project: [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts `.osm.pbf` files into the same tiles.

MIT license.
