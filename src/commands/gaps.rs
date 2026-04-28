use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::core::gaps::list_gaps;

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
