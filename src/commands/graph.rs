use std::collections::HashMap;
use std::io::{self, Write};

use anyhow::{anyhow, bail, Result};
use clap::{Args, Subcommand};
use rusqlite::Connection;

use crate::core::entities;
use crate::core::graph::{self, GraphError, TemporalFilter};
use crate::core::pages;
use crate::core::vault_sync;

/// Top-level args for the `quaid graph` command.
///
/// `quaid graph <slug>` keeps its existing single-page neighbourhood behaviour.
/// `quaid graph extract-entities` opts in to the bulk entity-pattern backfill
/// (Wave 5 / task 8.1) without performing it automatically as part of any
/// schema initialisation path.
#[derive(Args)]
pub struct GraphArgs {
    /// Optional subcommand. Defaults to neighbourhood rendering for `slug`.
    #[command(subcommand)]
    pub action: Option<GraphAction>,

    /// Page slug for the default neighbourhood view.
    pub slug: Option<String>,

    /// Maximum BFS depth for the neighbourhood view.
    #[arg(long, default_value = "2")]
    pub depth: u32,

    /// Temporal filter: `current` (default) or `all`/`history`.
    #[arg(long, default_value = "current")]
    pub temporal: String,
}

/// Subcommands attached to `quaid graph`.
#[derive(Subcommand)]
pub enum GraphAction {
    /// Opt-in backfill: run entity-pattern extraction across every page.
    ///
    /// This command is NEVER invoked by schema init or schema-mismatch
    /// handling; users must opt in explicitly. Idempotent on re-run.
    ExtractEntities,
}

/// CLI entry point dispatched from `main.rs`.
pub fn run_cli(db: &Connection, args: GraphArgs, json: bool) -> Result<()> {
    match args.action {
        Some(GraphAction::ExtractEntities) => run_extract_entities(db, json, &mut io::stdout()),
        None => {
            let slug = args
                .slug
                .ok_or_else(|| anyhow!("slug is required when no subcommand is given"))?;
            run(db, &slug, args.depth, &args.temporal, json)
        }
    }
}

/// Run the `quaid graph` command, writing output to stdout.
pub fn run(db: &Connection, slug: &str, depth: u32, temporal: &str, json: bool) -> Result<()> {
    run_to(db, slug, depth, temporal, json, &mut io::stdout())
}

/// Run the `quaid graph` command, writing output to `out`.
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

    let resolved = vault_sync::resolve_page_for_read(db, slug)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    let page_id: i64 = pages::resolve(
        db,
        &pages::PageKey {
            collection_id: resolved.collection_id,
            namespace: None,
            slug: &resolved.slug,
        },
    )
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => {
            anyhow::anyhow!("page not found: {}", resolved.canonical_slug())
        }
        other => anyhow::anyhow!(other),
    })?;

    let root_slug = resolved.canonical_slug();
    let result = match graph::neighborhood_graph_for_page(
        page_id,
        &resolved.collection_name,
        &resolved.slug,
        depth,
        filter,
        db,
    ) {
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
        render_text_graph(out, &root_slug, &result)?;
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
    write_children(out, root_slug, &edges_by_from, 1, &mut active_path)?;

    // Append a path explanation block for every reachable non-root node so
    // operators can see exactly which edges connect a hit to the root.
    let mut sorted: Vec<_> = result.paths.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut printed_header = false;
    for (slug, path) in sorted {
        if path.is_empty() {
            continue;
        }
        if !printed_header {
            writeln!(out, "paths:")?;
            printed_header = true;
        }
        let chain = path
            .iter()
            .map(|(from, rel, to)| format!("{from} -[{rel}]-> {to}"))
            .collect::<Vec<_>>()
            .join("  ");
        writeln!(out, "  {slug}: {chain}")?;
    }
    Ok(())
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

/// Run `quaid graph extract-entities`: load patterns once, iterate every page,
/// and route matches to assertions. Writing-mode commands validate the active
/// pattern set up-front (task 7.6 — malformed YAML/regex/capture/weight fails
/// before any mutation).
pub fn run_extract_entities<W: Write>(db: &Connection, json: bool, out: &mut W) -> Result<()> {
    let patterns = entities::load_patterns(db).map_err(|err| anyhow!(err.to_string()))?;

    let mut stmt = db.prepare(
        "SELECT p.id, p.collection_id, p.slug, p.compiled_truth \
         FROM pages p \
         WHERE p.superseded_by IS NULL \
         ORDER BY p.id",
    )?;
    let rows: Vec<(i64, i64, String, String)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?
        .collect::<Result<_, _>>()?;

    let mut pages_seen = 0_usize;
    let mut assertions_inserted = 0_usize;
    let mut matches_total = 0_usize;
    let mut over_budget_pages = 0_usize;
    let mut errors = 0_usize;

    for (page_id, collection_id, slug, compiled_truth) in rows {
        pages_seen += 1;
        let outcome =
            entities::extract_entities(&compiled_truth, &patterns, entities::EXTRACTION_BUDGET);
        if outcome.over_budget {
            over_budget_pages += 1;
        }
        match entities::run_for_page(
            db,
            page_id,
            collection_id,
            &slug,
            &compiled_truth,
            &patterns,
        ) {
            Ok(summary) => {
                matches_total += summary.matches_seen;
                assertions_inserted += summary.assertions_inserted;
            }
            Err(_) => {
                errors += 1;
            }
        }
    }

    if json {
        let summary = serde_json::json!({
            "pages_seen": pages_seen,
            "matches_total": matches_total,
            "assertions_inserted": assertions_inserted,
            "over_budget_pages": over_budget_pages,
            "errors": errors,
            "patterns_loaded": patterns.len(),
        });
        writeln!(out, "{}", serde_json::to_string_pretty(&summary)?)?;
    } else {
        writeln!(
            out,
            "quaid graph extract-entities\n  patterns_loaded:     {}\n  pages_seen:          {}\n  matches_total:       {}\n  assertions_inserted: {}\n  over_budget_pages:   {}\n  errors:              {}",
            patterns.len(),
            pages_seen,
            matches_total,
            assertions_inserted,
            over_budget_pages,
            errors,
        )?;
    }

    Ok(())
}
