//! The tile key: one u64 that holds a map tile (Web Mercator z/x/y) and its zoom.
//!
//! Bits 63..62 are 0. Bits 61..6 hold the quadkey digits, the coarsest first, left-aligned (digit = 2 * y bit + x bit).
//! Bits 5..0 hold the zoom. A parent sorts before its children, the descendants of a tile are one key range, and the
//! common ancestor of two tiles is a bit operation.

/// The zoom of the keys that duckOSM stores (`nodes.tile`, `edges.from_tile`, `edges.to_tile`).
pub const MAX_ZOOM: u8 = 28;

/// The key of the tile of zoom `z` that holds the point. Same formula as pbfcut `Tile::of`.
pub fn key(lat: f64, lon: f64, z: u8) -> u64 {
    let n = (1u64 << z) as f64;
    let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0) as u64;
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let y = ((1.0 - lat.tan().asinh() / std::f64::consts::PI) / 2.0 * n).floor().clamp(0.0, n - 1.0) as u64;
    let mut d = 0u64;
    for i in (0..z).rev() {
        d = (d << 2) | (((y >> i) & 1) << 1) | ((x >> i) & 1);
    }
    (d << shift(z)) | z as u64
}

fn shift(z: u8) -> u32 {
    6 + 2 * (MAX_ZOOM - z) as u32
}

pub fn zoom(k: u64) -> u8 {
    (k & 63) as u8
}

/// The ancestor of zoom `r` (the key itself if `r` is not coarser).
pub fn parent(k: u64, r: u8) -> u64 {
    if r >= zoom(k) {
        return k;
    }
    (k & !((1u64 << shift(r)) - 1)) | r as u64
}

/// The x and the y of a tile.
pub fn xy(k: u64) -> (u64, u64) {
    let d = k >> shift(zoom(k));
    let (mut x, mut y) = (0, 0);
    for i in 0..zoom(k) {
        let q = (d >> (2 * i)) & 3;
        x |= (q & 1) << i;
        y |= (q >> 1) << i;
    }
    (x, y)
}

/// The zoom of the lowest common ancestor of two tiles.
pub fn lca_zoom(a: u64, b: u64) -> u8 {
    let m = zoom(a).min(zoom(b));
    let d = (a ^ b) >> 6;
    if d == 0 {
        return m;
    }
    (((d.leading_zeros() - 8) / 2) as u8).min(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monaco_key_and_parent() {
        // the test vector sent to duckOSM: x=133 y=93 at zoom 8
        let k8 = key(43.7384, 7.4246, 8);
        let k28 = key(43.7384, 7.4246, 28);
        assert_eq!(k8, 1778007059137036296);
        assert_eq!(k28, 1778020963338276700);
        assert_eq!(parent(k28, 8), k8);
        assert_eq!(xy(k8), (133, 93));
        assert_eq!(parent(k28, 0), 0);
        assert_eq!(lca_zoom(k28, k8), 8);
        assert_eq!(lca_zoom(k28, k28), 28);
        // a point 1 km away shares a tile up to about zoom 14 to 15
        let z = lca_zoom(k28, key(43.7474, 7.4246, 28));
        assert!((12..=16).contains(&z), "{z}");
        // the other side of the world: only zoom 0
        assert_eq!(lca_zoom(k28, key(-33.0, -70.0, 28)), 0);
    }
}
