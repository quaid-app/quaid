use std::collections::HashMap;
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
        render_text_graph(out, slug, &result)?;
    }

    Ok(())
}

fn render_text_graph<W: Write>(
    out: &mut W,
    root_slug: &str,
    result: &graph::GraphResult,
) -> io::Result<()> {
    writeln!(out, "{root_slug}")?;

    let mut edges_by_from: HashMap<&str, Vec<&graph::GraphEdge>> = HashMap::new();
    for edge in &result.edges {
        // Defense-in-depth: never render a self-link as a neighbour.
        if edge.from == edge.to {
            continue;
        }
        edges_by_from
            .entry(edge.from.as_str())
            .or_default()
            .push(edge);
    }

    let mut active_path = vec![root_slug];
    write_children(out, root_slug, &edges_by_from, 1, &mut active_path)
}

fn write_children<'a, W: Write>(
    out: &mut W,
    parent_slug: &'a str,
    edges_by_from: &HashMap<&'a str, Vec<&'a graph::GraphEdge>>,
    depth: usize,
    active_path: &mut Vec<&'a str>,
) -> io::Result<()> {
    let Some(edges) = edges_by_from.get(parent_slug) else {
        return Ok(());
    };

    for edge in edges {
        let child_slug = edge.to.as_str();
        if active_path.contains(&child_slug) {
            continue;
        }

        writeln!(
            out,
            "{}→ {} ({})",
            "  ".repeat(depth),
            edge.to,
            edge.relationship
        )?;

        active_path.push(child_slug);
        write_children(out, child_slug, edges_by_from, depth + 1, active_path)?;
        active_path.pop();
    }

    Ok(())
}
