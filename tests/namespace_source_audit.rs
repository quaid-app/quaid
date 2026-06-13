#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Source-text audit pinning namespace-aware page identity (issue #212).
//!
//! Every production SQL lookup that matches `slug = ?` against the `pages`
//! table must either go through `core::pages::resolve*` or carry a namespace
//! predicate in the same statement. Sites that are deliberately
//! namespace-blind are allowlisted below with a documented reason; the test
//! fails both on NEW unlisted sites (drift) and on STALE allowlist entries
//! (so the list shrinks as sites migrate).

use std::path::{Path, PathBuf};

/// Deliberately namespace-blind lookups, with reasons.
const ALLOWLIST: &[(&str, usize, &str)] = &[
    (
        "src/core/collections.rs",
        1,
        "slug -> owning-collection resolution: namespaces partition pages, not collections",
    ),
    (
        "src/core/entities.rs",
        1,
        "basename surface matcher with LIMIT 2 ambiguity guard; multi-match resolves to Unresolved",
    ),
    (
        "src/core/quarantine.rs",
        1,
        "quarantined-page load is collection-scoped and predicated on quarantined_at",
    ),
    (
        "src/core/search.rs",
        1,
        "link-frontier expansion over collection-scoped result slugs; namespace filter applies to the seed results upstream",
    ),
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read src dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Strip the trailing `#[cfg(test)] mod ...` block; fn-level `#[cfg(test)]`
/// items (e.g. in search.rs) are kept because production code follows them.
fn production_source(source: &str) -> &str {
    let mut search_from = 0;
    while let Some(offset) = source[search_from..].find("#[cfg(test)]") {
        let start = search_from + offset;
        let rest = source[start + "#[cfg(test)]".len()..].trim_start();
        if rest.starts_with("mod") {
            return &source[..start];
        }
        search_from = start + "#[cfg(test)]".len();
    }
    source
}

fn violation_count(production: &str) -> usize {
    let needle = "slug = ?";
    let mut count = 0;
    let mut search_from = 0;
    while let Some(offset) = production[search_from..].find(needle) {
        let index = search_from + offset;
        search_from = index + needle.len();

        // Identifier suffix matches like `resolved_by_slug = ?1` are not
        // pages-slug lookups.
        let preceding = production[..index].chars().next_back().unwrap_or(' ');
        if preceding.is_alphanumeric() || preceding == '_' {
            continue;
        }
        // `UPDATE pages SET slug = ?` assigns a slug to an id-keyed row; it
        // is not an identity lookup.
        if production[..index].trim_end().ends_with("SET") {
            continue;
        }

        let window_start = index.saturating_sub(600);
        let window_end = (index + 600).min(production.len());
        let window = &production[window_start..window_end];
        if !window.contains("pages") {
            continue;
        }
        if window.contains("namespace") {
            continue;
        }
        count += 1;
    }
    count
}

#[test]
fn pages_slug_lookups_outside_core_pages_carry_a_namespace_predicate() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let src_root = Path::new(&manifest_dir).join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_root, &mut files);
    files.sort();
    assert!(
        files.len() > 50,
        "audit walked suspiciously few files: {}",
        files.len()
    );

    let mut unexpected: Vec<String> = Vec::new();
    let mut stale: Vec<String> = Vec::new();

    for file in &files {
        let relative = file
            .strip_prefix(&manifest_dir)
            .expect("path under manifest dir")
            .to_string_lossy()
            .replace('\\', "/");
        if relative == "src/core/pages.rs" {
            // The sanctioned resolver implementation.
            continue;
        }

        let source = std::fs::read_to_string(file).expect("read source file");
        let count = violation_count(production_source(&source));

        match ALLOWLIST
            .iter()
            .find(|(allowed, _, _)| *allowed == relative)
        {
            Some((_, allowed_count, reason)) => {
                if count > *allowed_count {
                    unexpected.push(format!(
                        "{relative}: {count} namespace-blind pages-slug lookups \
                         (allowlist permits {allowed_count}: {reason})"
                    ));
                } else if count < *allowed_count {
                    stale.push(format!(
                        "{relative}: allowlist permits {allowed_count} but found {count}; \
                         shrink the allowlist entry"
                    ));
                }
            }
            None => {
                if count > 0 {
                    unexpected.push(format!(
                        "{relative}: {count} namespace-blind pages-slug lookups; \
                         migrate to core::pages::resolve or add a namespace predicate"
                    ));
                }
            }
        }
    }

    assert!(
        unexpected.is_empty(),
        "namespace-blind `slug = ?` lookups against pages detected:\n{}",
        unexpected.join("\n")
    );
    assert!(
        stale.is_empty(),
        "stale namespace source-audit allowlist entries:\n{}",
        stale.join("\n")
    );
}
