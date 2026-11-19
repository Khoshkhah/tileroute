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
pub fn read(db: &Path, mode: &str, finest: u8) -> Result<Network> {
    let con = Connection::open_with_flags(db, Config::default().access_mode(AccessMode::ReadOnly).map_err(err)?)
        .map_err(err)?;
    // duckOSM routed_graph: the rows of the mode's own use
    let relation = if has_column(&con, mode, "edge_graph", "uses")? {
        format!("(SELECT * FROM {mode}.edge_graph WHERE uses IN ('{}'))", routed_use(mode)?)
    } else {
        format!("{mode}.edge_graph")
    };
    let mut b = GraphBuilder::default();
    {
        let mut st = con.prepare(&format!("SELECT from_edge, to_edge, cost::DOUBLE FROM {relation}")).map_err(err)?;
        let mut rows = st.query([]).map_err(err)?;
        while let Some(r) = rows.next().map_err(err)? {
            let w: Option<f64> = r.get(2).map_err(err)?;
            b.add_arc(r.get(0).map_err(err)?, r.get(1).map_err(err)?, w.unwrap_or(f64::NAN));
        }
    }
    let graph = b.finish();
    let turns: Vec<(u32, u32, f64)> = graph
        .arcs()
        .filter(|a| !a.2.is_nan())
        .map(|(u, w, c)| (graph.index_of(u).expect("a node") as u32, graph.index_of(w).expect("a node") as u32, c))
        .collect();

    // the tiles of the two end points of each edge: duckOSM's columns, or from the node points
    let sql = if has_column(&con, mode, "edges", "from_tile")? {
        format!("SELECT edge_id, from_tile, to_tile FROM {mode}.edges")
    } else {
        con.execute_batch("LOAD spatial;").map_err(err)?;
        format!(
            "SELECT e.edge_id, ST_Y(a.geom), ST_X(a.geom), ST_Y(b.geom), ST_X(b.geom) FROM {mode}.edges e
             JOIN {mode}.nodes a ON a.node_id = e.source JOIN {mode}.nodes b ON b.node_id = e.target"
        )
    };
    let computed = !sql.contains("from_tile");
    let n = graph.n_nodes();
    // a node with no tile (not in edges): level 0, the whole world
    let mut level = vec![0u8; n];
    let mut cell = vec![0u64; n];
    let mut st = con.prepare(&sql).map_err(err)?;
    let mut rows = st.query([]).map_err(err)?;
    while let Some(r) = rows.next().map_err(err)? {
        let id: i64 = r.get(0).map_err(err)?;
        let Some(i) = graph.index_of(id) else { continue };
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
        if let (Some(a), Some(b)) = (a, b) {
            let z = tile::lca_zoom(a, b).min(finest);
            level[i] = z;
            cell[i] = tile::parent(a, z);
        }
    }
    Ok(Network { graph, turns, level, cell })
}
