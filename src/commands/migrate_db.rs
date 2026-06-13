#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

//! `quaid migrate` — explicit, opt-in schema migration. Walks the versioned
//! migration ladder in [`crate::core::db::migrate_database`] after writing a
//! `.bak` backup, then reports integrity and row-count results. Named
//! `migrate_db` to avoid colliding with [`crate::core::migrate`], the
//! export/import module.

use anyhow::Result;
use serde_json::json;

use crate::core::db;

/// Migrate the database at `path` to the current schema version and print a
/// report (human-readable, or JSON when `json_output` is set).
pub fn run(path: &str, json_output: bool) -> Result<()> {
    let report = db::migrate_database(path)?;

    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "from_version": report.from_version,
                "to_version": report.to_version,
                "steps_applied": report.steps_applied,
                "backup_path": report.backup_path,
                "integrity_check": "ok",
                "pages": { "before": report.pages_before, "after": report.pages_after },
                "links": { "before": report.links_before, "after": report.links_after },
            }))?
        );
        return Ok(());
    }

    if report.steps_applied.is_empty() {
        println!(
            "Database at {path} is already at schema version {}; nothing to migrate.",
            report.to_version
        );
        return Ok(());
    }

    let steps = report
        .steps_applied
        .iter()
        .map(|version| format!("v{version}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "Migrated {path} from schema version {} to {}.",
        report.from_version, report.to_version
    );
    if let Some(backup) = &report.backup_path {
        println!("  Pre-migration backup: {backup}");
    }
    println!("  Steps applied: {steps}");
    println!("  Integrity check: ok");
    println!(
        "  Pages: {} before, {} after",
        report.pages_before, report.pages_after
    );
    println!(
        "  Links: {} before, {} after",
        report.links_before, report.links_after
    );
    Ok(())
}
