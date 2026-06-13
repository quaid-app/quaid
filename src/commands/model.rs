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
    download_model, download_model_pinned, inspect_model_caches, remove_model_cache_entries,
    resolve_model_alias, ConsoleProgressReporter, ModelCacheCleanReport, ModelCacheEntry,
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

pub fn run(
    action: ModelAction,
    allow_unverified_model: bool,
    model_revision: Option<&str>,
) -> Result<()> {
    match action {
        ModelAction::Pull { alias } => {
            let cache_dir = pull(&alias, allow_unverified_model, model_revision)?;
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

/// Resolve and download a model alias, enforcing the unpinned-download
/// policy *before* any network or download machinery runs: curated
/// aliases keep their authoritative pins (and refuse overrides), while
/// custom model ids require both `--allow-unverified-model` and an
/// explicit `--model-revision <commit-sha>`.
pub(crate) fn pull(
    alias: &str,
    allow_unverified_model: bool,
    model_revision: Option<&str>,
) -> Result<std::path::PathBuf> {
    let resolved = resolve_model_alias(alias)?;
    let mut progress = ConsoleProgressReporter::default();
    if resolved.revision.is_some() {
        if allow_unverified_model || model_revision.is_some() {
            bail!(
                "model `{alias}` is a curated alias with a pinned revision; \
                 --model-revision/--allow-unverified-model only apply to custom model ids"
            );
        }
        return Ok(tokio::task::block_in_place(|| {
            download_model(alias, &mut progress)
        })?);
    }

    let Some(revision) = model_revision else {
        bail!(
            "refusing to download unpinned model `{alias}`: custom models have no curated \
             SHA/revision pin, and Quaid will not silently fetch the mutable `main` revision. \
             Re-run with --allow-unverified-model and --model-revision <commit-sha> to pin \
             the download explicitly."
        );
    };
    if !allow_unverified_model {
        bail!(
            "custom model `{alias}` has no curated SHA-256 pin, so its files cannot be \
             integrity-verified. Re-run with --allow-unverified-model (alongside \
             --model-revision {revision}) to accept the unverified download."
        );
    }
    Ok(tokio::task::block_in_place(|| {
        download_model_pinned(alias, Some(revision), &mut progress)
    })?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::conversation::model_lifecycle::{ModelCacheFamily, ModelCacheState};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.as_ref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn entry(
        state: ModelCacheState,
        cleanup_eligible: bool,
        complete_cache: bool,
    ) -> ModelCacheEntry {
        ModelCacheEntry {
            family: ModelCacheFamily::Extraction,
            alias: Some("phi-3.5-mini".to_owned()),
            cache_key: format!("{state}"),
            path: std::path::PathBuf::from(format!("/tmp/{state}")),
            state,
            reason: state.to_string(),
            file_count: 0,
            size_bytes: 0,
            modified_unix: None,
            cleanup_eligible,
            complete_cache,
        }
    }

    #[test]
    fn cleanup_candidates_selects_safe_entries_for_each_mode() {
        let stale = entry(ModelCacheState::StaleTemporary, true, false);
        let complete = entry(ModelCacheState::Complete, false, true);
        let missing = entry(ModelCacheState::Missing, false, false);
        let entries = vec![stale.clone(), complete.clone(), missing];

        assert_eq!(
            cleanup_candidates(&entries, false, false),
            vec![stale.clone()]
        );
        assert_eq!(
            cleanup_candidates(&entries, false, true),
            vec![stale.clone()]
        );
        assert_eq!(
            cleanup_candidates(&entries, true, false),
            vec![stale, complete]
        );
    }

    #[test]
    fn human_formatters_use_binary_units_and_seconds() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(1024 * 1024), "1.0 MiB");
        assert_eq!(
            human_duration(std::time::Duration::from_millis(250)),
            "250 ms"
        );
        assert_eq!(
            human_duration(std::time::Duration::from_millis(1250)),
            "1.2 s"
        );
    }

    #[test]
    fn collect_cache_files_recurses_and_uses_relative_slash_paths() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let nested = dir.path().join("nested");
        std::fs::create_dir_all(&nested).expect("nested dir");
        std::fs::write(dir.path().join("root.bin"), [1_u8, 2]).expect("root file");
        std::fs::write(nested.join("child.bin"), [3_u8, 4, 5]).expect("child file");

        let mut files = Vec::new();
        collect_cache_files(dir.path(), dir.path(), &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));

        assert_eq!(
            files,
            vec![
                ("nested/child.bin".to_owned(), 3),
                ("root.bin".to_owned(), 2)
            ]
        );
    }

    #[test]
    fn print_helpers_accept_empty_and_populated_reports() {
        print_cache_entries(&[], "empty", false);
        print_cache_entries(
            &[entry(ModelCacheState::Complete, false, true)],
            "empty",
            true,
        );
        print_clean_report(&ModelCacheCleanReport {
            removed: vec![
                crate::core::conversation::model_lifecycle::ModelCacheRemoval {
                    path: std::path::PathBuf::from("/tmp/removed"),
                    size_bytes: 2048,
                    error: None,
                },
            ],
            failed: vec![
                crate::core::conversation::model_lifecycle::ModelCacheRemoval {
                    path: std::path::PathBuf::from("/tmp/failed"),
                    size_bytes: 0,
                    error: Some("nope".to_owned()),
                },
            ],
            bytes_freed: 2048,
        });
    }

    #[test]
    fn run_status_and_clean_list_paths_do_not_require_downloads() {
        run(
            ModelAction::Status {
                alias: Some("phi-3.5-mini".to_owned()),
                verbose: false,
                verify: false,
            },
            false,
            None,
        )
        .expect("status should inspect local cache only");

        clean(None, false, false, false).expect("default clean lists candidates only");

        let error = clean(Some("phi-3.5-mini"), false, true, false)
            .expect_err("alias and all are mutually exclusive");
        assert!(error.to_string().contains("either an alias or --all"));
    }

    #[test]
    fn run_pull_and_verified_status_paths_do_not_download_without_feature() {
        if cfg!(feature = "online-model") {
            return;
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let error = runtime
            .block_on(async {
                run(
                    ModelAction::Pull {
                        alias: "phi-3.5-mini".to_owned(),
                    },
                    false,
                    None,
                )
            })
            .expect_err("offline build reports unsupported downloads");
        assert!(error.to_string().contains("online-model"));

        run(
            ModelAction::Status {
                alias: Some("phi-3.5-mini".to_owned()),
                verbose: false,
                verify: true,
            },
            false,
            None,
        )
        .expect("verified status should inspect local cache only");
    }

    #[test]
    fn clean_force_removes_alias_cache_from_temp_root() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let _cache_root = EnvVarGuard::set("QUAID_MODEL_CACHE_DIR", cache_root.path());
        let cache_dir = cache_root.path().join("org-test-model");
        std::fs::create_dir(&cache_dir).expect("cache dir");

        run(
            ModelAction::Clean {
                alias: Some("org/test-model".to_owned()),
                list: false,
                all: false,
                force: true,
            },
            false,
            None,
        )
        .expect("clean removes corrupt alias cache");

        assert!(!cache_dir.exists());
    }
}
