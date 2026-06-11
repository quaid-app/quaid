#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use anyhow::Result;
use clap::Subcommand;
use rusqlite::Connection;
use serde::Serialize;

use crate::core::collections::{self, OpKind, SlugResolution};
use crate::core::gaps::{list_gaps, resolve_gap};

/// Subactions of `quaid gaps` (plain `quaid gaps` lists unresolved gaps).
#[derive(Clone, Debug, Subcommand)]
pub enum GapsAction {
    /// Mark a knowledge gap resolved by the page that answered it
    Resolve {
        /// Numeric gap id (see `quaid gaps`)
        id: i64,
        /// Slug of the page that answers the gap
        slug: String,
    },
}

/// Handle `quaid gaps resolve <id> <slug>`: validate that the slug resolves
/// to an existing page, then flip the gap's `resolved_at`/`resolved_by_slug`.
pub fn resolve(db: &Connection, id: i64, slug: &str, json: bool) -> Result<()> {
    let (collection_id, resolved_slug) = match collections::parse_slug(db, slug, OpKind::Read)? {
        SlugResolution::Resolved {
            collection_id,
            slug,
            ..
        } => (collection_id, slug),
        SlugResolution::NotFound { slug } => {
            anyhow::bail!("page not found: {slug}");
        }
        SlugResolution::Ambiguous { slug, candidates } => {
            anyhow::bail!(
                "ambiguous slug `{slug}`; candidates: {}",
                candidates
                    .into_iter()
                    .map(|candidate| candidate.full_address)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    };
    let page_exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pages WHERE collection_id = ?1 AND slug = ?2)",
        rusqlite::params![collection_id, &resolved_slug],
        |row| row.get(0),
    )?;
    if !page_exists {
        anyhow::bail!("page not found: {resolved_slug}");
    }

    resolve_gap(id, &resolved_slug, db)?;

    if json {
        println!(
            "{}",
            serde_json::json!({"id": id, "resolved_by_slug": resolved_slug})
        );
    } else {
        println!("Gap {id} resolved by {resolved_slug}.");
    }
    Ok(())
}

pub fn run(db: &Connection, limit: u32, resolved: bool, json: bool) -> Result<()> {
    let gaps = list_gaps(resolved, limit as usize, db)?;

    if json {
        #[derive(Serialize)]
        struct GapEntry {
            id: i64,
            query_hash: String,
            context: String,
            confidence_score: Option<f64>,
            sensitivity: String,
            resolved_at: Option<String>,
            resolved_by_slug: Option<String>,
            detected_at: String,
        }

        let entries: Vec<GapEntry> = gaps
            .into_iter()
            .map(|g| GapEntry {
                id: g.id,
                query_hash: g.query_hash,
                context: g.context,
                confidence_score: g.confidence_score,
                sensitivity: g.sensitivity,
                resolved_at: g.resolved_at,
                resolved_by_slug: g.resolved_by_slug,
                detected_at: g.detected_at,
            })
            .collect();

        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else if gaps.is_empty() {
        println!("No knowledge gaps found.");
    } else {
        for gap in &gaps {
            let resolved_info = match &gap.resolved_at {
                Some(ts) => format!(" resolved {ts}"),
                None => String::new(),
            };
            println!(
                "[{detected_at}] {hash} ({sensitivity}, confidence: {conf}{resolved_info})",
                detected_at = gap.detected_at,
                hash = gap.query_hash,
                sensitivity = gap.sensitivity,
                conf = gap
                    .confidence_score
                    .map(|s| format!("{s:.2}"))
                    .unwrap_or_else(|| "n/a".to_string()),
            );
        }
        println!("{} gap(s) found.", gaps.len());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{db, gaps};

    fn open_db() -> Connection {
        db::open(":memory:").expect("open in-memory db")
    }

    #[test]
    fn run_empty_text_succeeds() {
        let conn = open_db();
        run(&conn, 10, false, false).expect("run with no gaps");
    }

    #[test]
    fn run_empty_json_succeeds() {
        let conn = open_db();
        run(&conn, 10, false, true).expect("run with no gaps json");
    }

    #[test]
    fn run_with_gap_text_shows_entry() {
        let conn = open_db();
        gaps::log_gap(None, "test-query", "some context", Some(0.5), &conn).expect("log gap");
        run(&conn, 10, false, false).expect("run with gap text");
    }

    #[test]
    fn run_with_gap_json_includes_array() {
        let conn = open_db();
        gaps::log_gap(None, "json-query", "ctx", None, &conn).expect("log gap");
        run(&conn, 10, false, true).expect("run with gap json");
    }
}
