use anyhow::Result;
use clap::Subcommand;
use rusqlite::{params, Connection};

use crate::core::{
    conversation::model_lifecycle::{cached_model_status, download_model, ConsoleProgressReporter},
    db,
};

#[derive(Subcommand)]
pub enum ExtractionAction {
    /// Enable SLM extraction and eagerly cache the configured model
    Enable,
    /// Disable SLM extraction without deleting cached model files
    Disable,
    /// Show current extraction configuration and cache state
    Status,
}

pub fn run(db: &Connection, action: ExtractionAction) -> Result<()> {
    match action {
        ExtractionAction::Enable => enable(db),
        ExtractionAction::Disable => disable(db),
        ExtractionAction::Status => status(db),
    }
}

fn enable(db: &Connection) -> Result<()> {
    let alias = db::read_config_value_or(db, "extraction.model_alias", "phi-3.5-mini")?;
    let mut progress = ConsoleProgressReporter;
    let cache_dir = tokio::task::block_in_place(|| download_model(&alias, &mut progress))?;
    set_extraction_enabled(db, true)?;
    println!("Extraction enabled: yes");
    println!("Model alias: {alias}");
    println!("Model cache: {}", cache_dir.display());
    Ok(())
}

fn disable(db: &Connection) -> Result<()> {
    set_extraction_enabled(db, false)?;
    println!("Extraction enabled: no");
    Ok(())
}

fn status(db: &Connection) -> Result<()> {
    let enabled = extraction_enabled(db)?;
    let alias = db::read_config_value_or(db, "extraction.model_alias", "phi-3.5-mini")?;
    let cache_status = cached_model_status(&alias)?;
    let (pending, running, failed_recent) = queue_counts(db)?;

    println!("Extraction enabled: {}", yes_no(enabled));
    println!("Model alias: {}", cache_status.alias.requested_alias);
    println!("Model repo: {}", cache_status.alias.repo_id);
    println!("Model cache dir: {}", cache_status.cache_dir.display());
    println!(
        "Model cache state: {}",
        match (cache_status.is_cached, cache_status.verified) {
            (true, true) => "verified",
            (true, false) => "present but invalid",
            (false, _) => "missing",
        }
    );
    println!("Queue: pending={pending} running={running} failed_last_24h={failed_recent}");
    Ok(())
}

fn queue_counts(db: &Connection) -> Result<(i64, i64, i64)> {
    let counts = db.query_row(
        "SELECT
             COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE WHEN status = 'running' THEN 1 ELSE 0 END), 0),
             COALESCE(SUM(CASE
                 WHEN status = 'failed'
                  AND julianday(enqueued_at) >= julianday('now', '-1 day')
                 THEN 1
                 ELSE 0
             END), 0)
         FROM extraction_queue",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    Ok(counts)
}

fn extraction_enabled(db: &Connection) -> Result<bool> {
    Ok(matches!(
        db::read_config_value_or(db, "extraction.enabled", "false")?.as_str(),
        "true"
    ))
}

fn set_extraction_enabled(db: &Connection, enabled: bool) -> Result<()> {
    db.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('extraction.enabled', ?1)",
        params![if enabled { "true" } else { "false" }],
    )?;
    Ok(())
}

fn yes_no(value: bool) -> &'static str {
    if value {
        "yes"
    } else {
        "no"
    }
}
