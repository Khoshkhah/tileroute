//! Read a duckOSM database: the routing graph of a mode (the same rows and the same order as duckOSM's router), and the
//! level and the cell of each node from the tiles of its two end points.

use crate::tile;
use duckdb::{AccessMode, Config, Connection};
use duckosm_router::graph::{Graph, GraphBuilder};
use std::path::Path;

pub type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The routing graph and the tiles of one mode.
pub struct Network {
    /// duckOSM's graph (networkx order): the reference, and the node ids
    pub graph: Graph,
    /// the turns `(from, to, cost)` by node number, each pair once (the last weight, as networkx)
    pub turns: Vec<(u32, u32, f64)>,
    pub level: Vec<u8>,
    pub cell: Vec<u64>,
    /// the zoom-8 XYZ tile of the first junction of each road edge (only with the partition; else 0)
    pub home: Vec<u64>,
}

/// The own `uses` of a mode (duckOSM `ROUTED_USES`).
fn routed_use(mode: &str) -> Result<&'static str> {
    match mode {
        "driving" => Ok("car"),
        "walking" => Ok("walk"),
        "cycling" => Ok("bike"),
        m => Err(format!("unknown mode {m}")),
    }
}

fn has_column(con: &Connection, mode: &str, table: &str, column: &str) -> Result<bool> {
    let n: i64 = con
        .query_row(
            "SELECT count(*) FROM information_schema.columns WHERE table_schema = ? AND table_name = ? AND column_name = ?",
            [mode, table, column],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(n > 0)
}

/// Read the network of `mode`. `finest`: the finest level (a node of a higher `lca_zoom` gets this level).
/// Open a duckOSM database, read-only.
pub fn open(db: &Path) -> Result<Connection> {
    Connection::open_with_flags(db, Config::default().access_mode(AccessMode::ReadOnly).map_err(err)?).map_err(err)
}

/// The rows of `<mode>.edge_graph` that duckOSM's router uses (`routed_graph`), as a FROM-able relation.
pub fn relation(con: &Connection, mode: &str) -> Result<String> {
    Ok(if has_column(con, mode, "edge_graph", "uses")? {
        format!("(SELECT * FROM {mode}.edge_graph WHERE uses IN ('{}'))", routed_use(mode)?)
    } else {
        format!("{mode}.edge_graph")
    })
}

/// Call `f(edge_id, level, cell)` for each road edge of `<mode>.edges`; an edge with no tiles gets level 0 and cell 0
/// (the whole world). The tiles come from duckOSM's columns, or from the node points.
pub fn edge_levels(con: &Connection, mode: &str, finest: u8, mut f: impl FnMut(i64, u8, u64)) -> Result<()> {
    let sql = if has_column(con, mode, "edges", "from_tile")? {
        format!("SELECT edge_id, from_tile, to_tile FROM {mode}.edges")
    } else {
        con.execute_batch("LOAD spatial;").map_err(err)?;
        format!(
            "SELECT e.edge_id, ST_Y(a.geom), ST_X(a.geom), ST_Y(b.geom), ST_X(b.geom) FROM {mode}.edges e
             JOIN {mode}.nodes a ON a.node_id = e.source JOIN {mode}.nodes b ON b.node_id = e.target"
        )
    };
    let computed = !sql.contains("from_tile");
    let mut st = con.prepare(&sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let id: i64 = r.get(0).map_err(err)?;
        let (a, b): (Option<u64>, Option<u64>) = if computed {
            let p: [Option<f64>; 4] = [r.get(1).map_err(err)?, r.get(2).map_err(err)?, r.get(3).map_err(err)?, r.get(4).map_err(err)?];
            match p {
                [Some(la), Some(lo), Some(lb), Some(lob)] => {
                    (Some(tile::key(la, lo, tile::MAX_ZOOM)), Some(tile::key(lb, lob, tile::MAX_ZOOM)))
                }
                _ => (None, None),
            }
        } else {
            let a: Option<i64> = r.get(1).map_err(err)?;
            let b: Option<i64> = r.get(2).map_err(err)?;
            (a.map(|x| x as u64), b.map(|x| x as u64))
        };
        match (a, b) {
            (Some(a), Some(b)) => {
                let z = tile::lca_zoom(a, b).min(finest);
                f(id, z, tile::parent(a, z));
            }
            _ => f(id, 0, 0),
        }
    }
    Ok(())
}

pub fn read(db: &Path, mode: &str, finest: u8) -> Result<Network> {
    read_with(db, mode, finest, None)
}

/// Call `f(edge_id, level, cell)` for each road edge of `<mode>.edges`, with the cells from the inertial flow
/// partition of the junctions (`crate::part`) in place of the tiles: cut until a cell has at most `leaf` junctions or
/// the zoom is `finest`. The last value of `f` is the zoom-8 XYZ tile of the first junction of the road edge. Returns the
/// number of road edges in the cuts of each zoom.
pub fn partition_levels(con: &Connection, mode: &str, finest: u8, leaf: usize, mut f: impl FnMut(i64, u8, u64, u64)) -> Result<Vec<usize>> {
    con.execute_batch("LOAD spatial;").map_err(err)?;
    let mut index = std::collections::HashMap::new();
    let mut pos: Vec<(f64, f64)> = Vec::new();
    {
        let mut st = con.prepare(&format!("SELECT node_id, ST_X(geom), ST_Y(geom) FROM {mode}.nodes")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let id: i64 = r.get(0).map_err(err)?;
            let (x, y): (Option<f64>, Option<f64>) = (r.get(1).map_err(err)?, r.get(2).map_err(err)?);
            index.insert(id, pos.len() as u32);
            pos.push((x.unwrap_or(0.0), y.unwrap_or(0.0)));
        }
    }
    let xyz: Vec<u64> = pos.iter().map(|&(lon, lat)| tile::key(lat, lon, 8)).collect();
    // a plane: longitude times the cosine of the mean latitude, and latitude
    let lat0 = (pos.iter().map(|p| p.1).sum::<f64>() / pos.len().max(1) as f64).to_radians().cos();
    pos.iter_mut().for_each(|p| p.0 *= lat0);
    let mut ids = Vec::new();
    let mut ends = Vec::new();
    {
        let mut st = con.prepare(&format!("SELECT edge_id, source, target FROM {mode}.edges")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let (id, a, b): (i64, i64, i64) = (r.get(0).map_err(err)?, r.get(1).map_err(err)?, r.get(2).map_err(err)?);
            if let (Some(&a), Some(&b)) = (index.get(&a), index.get(&b)) {
                ids.push(id);
                ends.push((a, b));
            }
        }
    }
    let t = std::time::Instant::now();
    let adj = crate::part::Adjacency::new(pos.len(), &ends);
    let (key, cuts) = crate::part::partition(&pos, &adj, leaf, finest);
    eprintln!("partition: {} junctions, the cuts {:.2} s (the rest is the reading)", pos.len(), t.elapsed().as_secs_f64());
    for (id, &(a, b)) in ids.iter().zip(&ends) {
        let (ka, kb) = (key[a as usize], key[b as usize]);
        let z = tile::lca_zoom(ka, kb);
        f(*id, z, tile::parent(ka, z), xyz[a as usize]);
    }
    Ok(cuts)
}

/// duckOSM's graph of a mode (networkx order): the reference router.
pub fn graph(con: &Connection, mode: &str) -> Result<Graph> {
    let relation = relation(con, mode)?;
    let mut b = GraphBuilder::default();
    {
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {relation}")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let w: Option<f64> = r.get(2).map_err(err)?;
            b.add_arc(r.get(0).map_err(err)?, r.get(1).map_err(err)?, w.unwrap_or(f64::NAN));
        }
    }
    Ok(b.finish())
}

/// [`read`], with the cells from the partition when `leaf` is given (see [`partition_levels`]).
pub fn read_with(db: &Path, mode: &str, finest: u8, leaf: Option<usize>) -> Result<Network> {
    let con = open(db)?;
    let graph = graph(&con, mode)?;
    let turns: Vec<(u32, u32, f64)> = graph
        .arcs()
        .filter(|a| !a.2.is_nan())
        .map(|(u, w, c)| (graph.index_of(u).expect("a node") as u32, graph.index_of(w).expect("a node") as u32, c))
        .collect();
    let n = graph.n_nodes();
    // a node with no tile (not in edges): level 0, the whole world
    let mut level = vec![0u8; n];
    let mut cell = vec![0u64; n];
    let mut home = vec![0u64; n];
    let mut set = |id: i64, z: u8, c: u64, h: u64| {
        if let Some(i) = graph.index_of(id) {
            level[i] = z;
            cell[i] = c;
            home[i] = h;
        }
    };
    match leaf {
        Some(leaf) => {
            let t = std::time::Instant::now();
            let cuts = partition_levels(&con, mode, finest, leaf, &mut set)?;
            eprintln!("partition: {:.2} s, road edges in the cuts by zoom: {cuts:?}", t.elapsed().as_secs_f64());
        }
        None => edge_levels(&con, mode, finest, |id, z, c| set(id, z, c, 0))?,
    }
    Ok(Network { graph, turns, level, cell, home })
}
