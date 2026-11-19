# tileroute

Route on a road network that is stored by XYZ map tiles. A query reads from disk only the tiles that it needs, so a
country does not need all of its graph in memory. The same Rust code runs in the browser (WebAssembly).

Status: the shortcuts and the query work in memory, and each route equals duckOSM's router. The tile files on disk and the
browser version are not made yet.

Documentation: [docs/index.md](docs/index.md) (the site, built with `mkdocs build`).

| | |
|---|---|
| Input | A duckOSM database: `<mode>.edge_graph` and `<mode>.edges` |
| Method | Shortcuts by XYZ tile (a contraction hierarchy ordered by tiles), after h3-routing-platform |
| Routes | Equal to duckOSM's router: the same `edge_id`s and the same time |
| Files | One file for each zoom-8 tile, or one archive of all tiles |
| Runs on | Command line, Rust, Python (calls Rust), browser |

Sister project: [pbfcut](https://github.com/Khoshkhah/pbfcut) cuts `.osm.pbf` files into the same tiles.

MIT license.
