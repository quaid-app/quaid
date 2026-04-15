use std::io::{self, Write};

use anyhow::{bail, Result};
use rusqlite::Connection;

use crate::core::graph::{self, GraphError, TemporalFilter};

/// Run the `gbrain graph` command, writing output to stdout.
pub fn run(db: &Connection, slug: &str, depth: u32, temporal: &str, json: bool) -> Result<()> {
    run_to(db, slug, depth, temporal, json, &mut io::stdout())
}

/// Run the `gbrain graph` command, writing output to `out`.
///
/// Separated from `run` so tests can capture output without spawning a subprocess.
pub fn run_to<W: Write>(
    db: &Connection,
    slug: &str,
    depth: u32,
    temporal: &str,
    json: bool,
    out: &mut W,
) -> Result<()> {
    let filter = match temporal.to_lowercase().as_str() {
        "all" | "history" => TemporalFilter::All,
        _ => TemporalFilter::Active,
    };

    let result = match graph::neighborhood_graph(slug, depth, filter, db) {
        Ok(r) => r,
        Err(GraphError::PageNotFound { slug }) => {
            bail!("page not found: {slug}");
        }
        Err(GraphError::Sqlite(e)) => {
            return Err(e.into());
        }
    };

    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&result)?)?;
    } else {
        writeln!(out, "{slug}")?;
        for edge in &result.edges {
            writeln!(out, "  → {} ({})", edge.to, edge.relationship)?;
        }
    }

    Ok(())
}
