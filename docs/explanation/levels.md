# Tiles and levels

## The tile key

Each point gets the XYZ map tile (Web Mercator) that holds it, at zoom 28. A tile is stored as one 64-bit number, the
**tile key**:

| Bits | Holds |
|---|---|
| 63..62 | 0 (so the key is a positive BIGINT) |
| 61..6 | the quadkey digits, the coarsest first, left-aligned (digit = 2 × y bit + x bit) |
| 5..0 | the zoom (0 to 28) |

With this key:

| Operation | How |
|---|---|
| The parent at zoom r | Set the digits finer than r to 0, and put r in the low bits |
| The common ancestor of two tiles | The number of equal leading digits (`leading_zeros` of the XOR) |
| All tiles in a tile | One range of keys |

Zoom 28 is about 15 cm at the equator, so the key is exact for any road point. A coarser tile is one bit operation, so
the key is stored once, at zoom 28. duckOSM writes the same keys as columns: `nodes.tile`, `edges.from_tile`,
`edges.to_tile`, `edges.lca_zoom`.

## The level and the cell of a node

A node is a road edge, with two ends. Its **level** is the zoom of the smallest tile that holds both ends (`lca_zoom`).
Its **cell** is that tile.

| Road edge | Its two ends | Level | Cell |
|---|---|---|---|
| A short street | in the same small tile | high (for example 18) | a small tile |
| A long road | in two different large tiles | low (for example 9) | a large tile |
| A road across the border of zoom-1 tiles | in two halves of the world | 0 | the whole world |

The finest level is 20 (about 38 m at the equator). A road edge with a higher `lca_zoom` gets level 20.

!!! info "Why this is useful"
    A road edge of level z lies in its cell T (zoom z), but not in one child tile of T: it crosses the border between
    two children of T. Two road edges with the same level and different cells have no common junction, because their
    cells do not overlap. So the tiles of one zoom are independent, and tileroute does them in parallel.

## H3 resolution and XYZ zoom

h3-routing-platform uses H3 cells. Here are the zooms with about the same cell size, at the latitude of France (46° N):

| H3 resolution | H3 cell area | XYZ zoom with about the same area |
|---|---|---|
| 7 | 5.16 km² | 14 (2.9 km²) |
| 8 | 0.74 km² | 15 (0.72 km²) |
| 9 | 0.11 km² | 16 (0.18 km²) |
| 10 | 0.015 km² | 18 (0.011 km²) |
| 11 | 0.0021 km² | 19 (0.0028 km²) |
| 12 | 0.0003 km² | 21 (0.0002 km²) |
| 15 | 0.9 m² | 25 (0.7 m²) |

An H3 cell has 7 children that do not cover it exactly. An XYZ tile has 4 children that cover it exactly, and all tiles
have one common ancestor (zoom 0), so no special case is necessary.
