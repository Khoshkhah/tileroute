//! Write the hierarchy to tile files (the format is in `crate::disk`).

use super::Hierarchy;
use crate::disk::{self, AArc, SArc};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// What `write_files` wrote: (file name, nodes, bytes) for each file, the top file first, then `edges.idx`.
pub type Written = Vec<(String, u32, u64)>;

impl Hierarchy {
    /// Write the hierarchy to the folder `dir`: one file for each zoom-8 tile, the top file and the edge index.
    /// `ids`: the duckOSM edge id of each node (the node numbers of `load::Network`).
    pub fn write_files(&self, ids: &[i64], dir: &Path) -> io::Result<Written> {
        std::fs::create_dir_all(dir)?;
        let n = self.level.len();
        // the file of each node: key 0 (top) or its zoom-8 tile; in a file, the nodes by level, then by cell
        let mut groups: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
        for v in 0..n as u32 {
            let (l, c) = (self.level[v as usize], self.cell[v as usize]);
            groups.entry(if l <= disk::TOP { 0 } else { crate::tile::parent(c, disk::TOP + 1) }).or_default().push(v);
        }
        let mut num = vec![0u32; n];
        let mut files = Vec::new();
        let mut next = 0u32;
        for (&key, vs) in groups.iter_mut() {
            vs.sort_unstable_by_key(|&v| (self.level[v as usize], self.cell[v as usize], v));
            files.push((next, vs.len() as u32, key));
            for &v in vs.iter() {
                num[v as usize] = next;
                next += 1;
            }
        }
        let mut out = Written::new();
        for (&key, vs) in &groups {
            let first = num[vs[0] as usize];
            let level: Vec<u8> = vs.iter().map(|&v| self.level[v as usize]).collect();
            let cell: Vec<u64> = vs.iter().map(|&v| self.cell[v as usize]).collect();
            let edge: Vec<i64> = vs.iter().map(|&v| ids[self.to_old[v as usize] as usize]).collect();
            let search = |f: &super::Fast| -> (Vec<u32>, Vec<SArc>) {
                let mut off = vec![0u32];
                let mut arcs = Vec::new();
                for &v in vs {
                    arcs.extend(f.of(v).iter().map(|&(cost, w)| SArc { cost, other: num[w as usize], pad: 0 }));
                    off.push(arcs.len() as u32);
                }
                (off, arcs)
            };
            let (up_off, up) = search(&self.up);
            let (down_off, down) = search(&self.down);
            let (mut all_off, mut other, mut arc, mut paths) = (vec![0u32], Vec::new(), Vec::new(), Vec::new());
            for &v in vs {
                let r = self.all.off[v as usize] as usize..self.all.off[v as usize + 1] as usize;
                let mut row: Vec<(u32, AArc)> = r
                    .map(|j| {
                        let a = self.all.arc[j];
                        let start = paths.len() as u32;
                        paths.extend(self.paths[a.start as usize..(a.start + a.len) as usize].iter().map(|&x| num[x as usize]));
                        (num[self.all.other[j] as usize], AArc { cost: a.cost, start, len: a.len })
                    })
                    .collect();
                row.sort_unstable_by_key(|x| x.0);
                other.extend(row.iter().map(|x| x.0));
                arc.extend(row.iter().map(|x| x.1));
                all_off.push(other.len() as u32);
            }
            let b = disk::tile_bytes(
                first,
                vs.len() as u32,
                [
                    disk::bytes_of(&level),
                    disk::bytes_of(&cell),
                    disk::bytes_of(&edge),
                    disk::bytes_of(&up_off),
                    disk::bytes_of(&up),
                    disk::bytes_of(&down_off),
                    disk::bytes_of(&down),
                    disk::bytes_of(&all_off),
                    disk::bytes_of(&other),
                    disk::bytes_of(&arc),
                    disk::bytes_of(&paths),
                ],
            );
            let name = disk::file_name(key);
            std::fs::write(dir.join(&name), &b)?;
            out.push((name, vs.len() as u32, b.len() as u64));
        }
        let mut pairs: Vec<(i64, u32)> = (0..n).map(|v| (ids[self.to_old[v] as usize], num[v])).collect();
        pairs.sort_unstable();
        let b = disk::edges_bytes(&pairs);
        std::fs::write(dir.join("edges.idx"), &b)?;
        out.push(("edges.idx".into(), n as u32, b.len() as u64));
        let b = disk::index_bytes(&files);
        std::fs::write(dir.join("index.bin"), &b)?;
        out.push(("index.bin".into(), 0, b.len() as u64));
        Ok(out)
    }
}
