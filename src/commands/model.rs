#![expect(
    clippy::print_stdout,
    reason = "CLI command prints user-facing output to stdout by design"
)]

use std::io::{self, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Result};
use clap::Subcommand;

use crate::core::conversation::model_lifecycle::{
    download_model, inspect_model_caches, remove_model_cache_entries, ConsoleProgressReporter,
    ModelCacheCleanReport, ModelCacheEntry,
};

#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub enum ModelAction {
    /// Download a local extraction SLM into the Quaid model cache
    Pull { alias: String },
    /// Show model cache status for extraction and embedding models
    Status {
        /// Optional model alias or repo id to inspect
        alias: Option<String>,
        /// Show cache keys, mtimes, cleanup eligibility, and file paths
        #[arg(long)]
        verbose: bool,
        /// Verify file hashes instead of using fast metadata validation
        #[arg(long)]
        verify: bool,
    },
    /// Remove incomplete, stale, or explicitly selected model cache entries
    Clean {
        /// Optional model alias or repo id to clean
        alias: Option<String>,
        /// List removal candidates without deleting anything
        #[arg(long)]
        list: bool,
        /// Clean all stale temporary and invalid cache entries
        #[arg(long)]
        all: bool,
        /// Skip the interactive confirmation prompt
        #[arg(long)]
        force: bool,
    },
}

pub fn run(action: ModelAction) -> Result<()> {
    match action {
        ModelAction::Pull { alias } => {
            let mut progress = ConsoleProgressReporter::default();
            let cache_dir = tokio::task::block_in_place(|| download_model(&alias, &mut progress))?;
            println!("Model cached at {}", cache_dir.display());
            Ok(())
        }
        ModelAction::Status {
            alias,
            verbose,
            verify,
        } => {
            let started = Instant::now();
            let entries = inspect_model_caches(alias.as_deref(), verify)?;
            print_cache_entries(&entries, "No model caches found.", verbose);
            if verify {
                println!(
                    "Verification completed in {}.",
                    human_duration(started.elapsed())
                );
            }
            Ok(())
        }
        ModelAction::Clean {
            alias,
            list,
            all,
            force,
        } => clean(alias.as_deref(), list, all, force),
    }
}

fn clean(alias: Option<&str>, list: bool, all: bool, force: bool) -> Result<()> {
    if alias.is_some() && all {
        bail!("use either an alias or --all, not both");
    }

    let entries = inspect_model_caches(alias, false)?;
    let candidates = cleanup_candidates(&entries, alias.is_some(), all);
    if list || (!all && alias.is_none()) {
        println!("Would remove:");
        print_cache_entries(&candidates, "No cleanup candidates found.", false);
        if !all && alias.is_none() {
            println!("No caches removed. Use `quaid model clean --all` to remove these entries.");
        }
        return Ok(());
    }

    if candidates.is_empty() {
        println!("No cleanup candidates found.");
        return Ok(());
    }

    println!("Will remove:");
    print_cache_entries(&candidates, "No cleanup candidates found.", false);
    if !force && !confirm_cleanup(candidates.len(), all, alias)? {
        println!("No caches removed.");
        return Ok(());
    }

    let report = remove_model_cache_entries(&candidates);
    print_clean_report(&report);
    if !report.failed.is_empty() {
        bail!("failed to remove {} cache entrie(s)", report.failed.len());
    }
    Ok(())
}

fn cleanup_candidates(
    entries: &[ModelCacheEntry],
    alias_specific: bool,
    all: bool,
) -> Vec<ModelCacheEntry> {
    entries
        .iter()
        .filter(|entry| {
            if all {
                entry.cleanup_eligible
            } else if alias_specific {
                entry.cleanup_eligible || entry.complete_cache
            } else {
                entry.cleanup_eligible
            }
        })
        .cloned()
        .collect()
}

fn print_cache_entries(entries: &[ModelCacheEntry], empty_message: &str, verbose: bool) {
    if entries.is_empty() {
        println!("{empty_message}");
        return;
    }

    println!(
        "{:<10} {:<12} {:<14} {:>5} {:>10} {:>12}  Path",
        "Family", "Alias/Key", "State", "Files", "Size", "Modified"
    );
    for entry in entries {
        let alias_or_key = entry.alias.as_deref().unwrap_or(&entry.cache_key);
        println!(
            "{:<10} {:<12} {:<14} {:>5} {:>10} {:>12}  {}",
            entry.family,
            alias_or_key,
            entry.state,
            entry.file_count,
            human_bytes(entry.size_bytes),
            entry
                .modified_unix
                .map(|modified| modified.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            entry.path.display()
        );
        println!("  {}", entry.reason);
        if verbose {
            println!("  Cache key: {}", entry.cache_key);
            println!("  Cleanup eligible: {}", entry.cleanup_eligible);
            print_cache_files(&entry.path);
        }
    }
    let total = entries
        .iter()
        .fold(0_u64, |total, entry| total.saturating_add(entry.size_bytes));
    println!("Total size: {}", human_bytes(total));
}

fn print_clean_report(report: &ModelCacheCleanReport) {
    for removed in &report.removed {
        println!(
            "Removed {} ({})",
            removed.path.display(),
            human_bytes(removed.size_bytes)
        );
    }
    for failed in &report.failed {
        println!(
            "Failed {}: {}",
            failed.path.display(),
            failed.error.as_deref().unwrap_or("unknown error")
        );
    }
    println!("Freed {}", human_bytes(report.bytes_freed));
}

fn confirm_cleanup(count: usize, all: bool, alias: Option<&str>) -> Result<bool> {
    let scope = if all {
        "all stale or invalid model cache entries".to_owned()
    } else if let Some(alias) = alias {
        format!("cache entries for `{alias}`")
    } else {
        "selected model cache entries".to_owned()
    };
    eprint!("Remove {count} {scope}? [y/N] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0_usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn print_cache_files(path: &Path) {
    let mut files = Vec::new();
    collect_cache_files(path, path, &mut files);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    for (relative, size_bytes) in files {
        println!("  - {relative} ({})", human_bytes(size_bytes));
    }
}

fn collect_cache_files(root: &Path, current: &Path, files: &mut Vec<(String, u64)>) {
    let Ok(metadata) = std::fs::symlink_metadata(current) else {
        return;
    };
    if metadata.is_file() {
        let relative = current
            .strip_prefix(root)
            .ok()
            .and_then(|path| path.to_str())
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| current.to_str().unwrap_or("<non-utf8>"))
            .replace('\\', "/");
        files.push((relative, metadata.len()));
        return;
    }
    if !metadata.is_dir() {
        return;
    }
    let Ok(read_dir) = std::fs::read_dir(current) else {
        return;
    };
    for entry in read_dir.filter_map(Result::ok) {
        collect_cache_files(root, &entry.path(), files);
    }
}

fn human_duration(duration: std::time::Duration) -> String {
    let seconds = duration.as_secs_f64();
    if seconds < 1.0 {
        format!("{:.0} ms", seconds * 1000.0)
    } else {
        format!("{seconds:.1} s")
    }
}
