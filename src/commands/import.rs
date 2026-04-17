use std::path::Path;

use anyhow::Result;
use rusqlite::Connection;

use crate::core::migrate;
use crate::core::migrate::ImportStats;

/// Format the human-readable import summary line from stats.
pub fn format_import_summary(stats: &ImportStats) -> String {
    let total_skipped = stats.total_skipped();
    if total_skipped == 0 {
        format!("Imported {} page(s)", stats.imported)
    } else {
        let mut reasons = Vec::new();
        if stats.skipped_already_ingested > 0 {
            reasons.push(format!(
                "{} already ingested",
                stats.skipped_already_ingested
            ));
        }
        if stats.skipped_non_markdown > 0 {
            reasons.push(format!("{} non-markdown", stats.skipped_non_markdown));
        }
        format!(
            "Imported {} page(s) ({} skipped: {})",
            stats.imported,
            total_skipped,
            reasons.join(", ")
        )
    }
}

pub fn run(db: &Connection, path: &str, validate_only: bool) -> Result<()> {
    let dir = Path::new(path);
    let stats = migrate::import_dir(db, dir, validate_only)?;

    if validate_only {
        println!("Validation passed: {} file(s) OK", stats.imported);
    } else {
        println!("{}", format_import_summary(&stats));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_summary_zero_skips() {
        let stats = ImportStats {
            imported: 42,
            skipped_already_ingested: 0,
            skipped_non_markdown: 0,
        };
        assert_eq!(format_import_summary(&stats), "Imported 42 page(s)");
    }

    #[test]
    fn format_summary_single_reason_non_markdown() {
        let stats = ImportStats {
            imported: 10,
            skipped_already_ingested: 0,
            skipped_non_markdown: 3,
        };
        assert_eq!(
            format_import_summary(&stats),
            "Imported 10 page(s) (3 skipped: 3 non-markdown)"
        );
    }

    #[test]
    fn format_summary_single_reason_already_ingested() {
        let stats = ImportStats {
            imported: 0,
            skipped_already_ingested: 5,
            skipped_non_markdown: 0,
        };
        assert_eq!(
            format_import_summary(&stats),
            "Imported 0 page(s) (5 skipped: 5 already ingested)"
        );
    }

    #[test]
    fn format_summary_mixed_reasons() {
        let stats = ImportStats {
            imported: 440,
            skipped_already_ingested: 7,
            skipped_non_markdown: 1,
        };
        assert_eq!(
            format_import_summary(&stats),
            "Imported 440 page(s) (8 skipped: 7 already ingested, 1 non-markdown)"
        );
    }
}
