// Ignore pattern handling for vault sync.
//
// The `.gbrainignore` file on disk is authoritative. The `collections.ignore_patterns`
// DB column is a cached mirror. Atomic parsing ensures the mirror is only updated
// when the entire file is valid.

#![allow(dead_code)]

use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

// ── Built-in Defaults ─────────────────────────────────────────

/// Built-in ignore patterns applied to every collection.
/// User patterns in `.gbrainignore` are layered on top.
pub fn builtin_patterns() -> &'static [&'static str] {
    &[
        ".obsidian/**",
        ".git/**",
        "node_modules/**",
        "_templates/**",
        ".trash/**",
    ]
}

// ── Parse Error ───────────────────────────────────────────────

/// Structured parse error for a single line in `.gbrainignore`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IgnoreParseError {
    pub code: String,
    pub line: usize,
    pub raw: String,
    pub message: String,
}

impl IgnoreParseError {
    fn parse_error(line: usize, raw: String, message: String) -> Self {
        Self {
            code: "parse_error".to_owned(),
            line,
            raw,
            message,
        }
    }

    fn file_stably_absent() -> Self {
        Self {
            code: "file_stably_absent_but_clear_not_confirmed".to_owned(),
            line: 0,
            raw: String::new(),
            message: ".gbrainignore absent but prior mirror exists; use `gbrain collection ignore clear <name> --confirm` to clear explicitly".to_owned(),
        }
    }
}

// ── Atomic Parse ──────────────────────────────────────────────

/// Parse result: either a valid pattern set or a list of errors.
#[derive(Debug)]
pub enum ParseResult {
    Valid(Vec<String>),
    Invalid(Vec<IgnoreParseError>),
}

/// Atomic parse of `.gbrainignore`: validate every non-comment line before any effect.
///
/// Returns `Valid(patterns)` if all lines are valid or comment/blank.
/// Returns `Invalid(errors)` if any line fails `globset::Glob::new`.
pub fn parse_ignore_file(content: &str) -> ParseResult {
    let mut patterns = Vec::new();
    let mut errors = Vec::new();

    for (line_num, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        // Skip blank lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        match Glob::new(trimmed) {
            Ok(_) => patterns.push(trimmed.to_owned()),
            Err(e) => {
                errors.push(IgnoreParseError::parse_error(
                    line_num + 1,
                    line.to_owned(),
                    format!("Invalid glob pattern: {}", e),
                ));
            }
        }
    }

    if errors.is_empty() {
        ParseResult::Valid(patterns)
    } else {
        ParseResult::Invalid(errors)
    }
}

// ── Reload Patterns (sole writer of collections.ignore_patterns) ──

/// Reload patterns from `.gbrainignore` and update the DB mirror.
///
/// This is the SOLE writer of `collections.ignore_patterns`. Invoked by:
/// - The watcher on `.gbrainignore` events
/// - `gbrain serve` startup
/// - `gbrain collection ignore add|remove|clear` (after file write)
///
/// # Behavior
///
/// - File present + fully valid → refresh mirror, clear `ignore_parse_errors`, return Ok
/// - File present + any invalid line → mirror UNCHANGED, record errors, return Err
/// - File absent + no prior mirror → defaults only, return Ok (no warning here; caller logs)
/// - File absent + prior mirror present → mirror UNCHANGED, return Err with special error
///
/// Caller is responsible for logging warnings and triggering reconciliation after success.
pub fn reload_patterns(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
) -> Result<(), ReloadError> {
    let ignore_path = root_path.join(".gbrainignore");
    let file_exists = ignore_path.exists();

    // Check if there's a prior mirror
    let prior_mirror: Option<String> = conn
        .query_row(
            "SELECT ignore_patterns FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .map_err(|e| ReloadError::DbError(e.to_string()))?;

    if !file_exists {
        if prior_mirror.is_some() {
            // File stably absent but mirror exists → operator must explicitly clear
            let err = IgnoreParseError::file_stably_absent();
            conn.execute(
                "UPDATE collections SET ignore_parse_errors = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                [serde_json::to_string(std::slice::from_ref(&err)).unwrap(), collection_id.to_string()],
            )
            .map_err(|e| ReloadError::DbError(e.to_string()))?;
            return Err(ReloadError::FileStablyAbsent(err));
        }
        // No file, no prior mirror → defaults only (this is a clean initial state)
        return Ok(());
    }

    // File exists → parse it
    let content = fs::read_to_string(&ignore_path)
        .map_err(|e| ReloadError::IoError(format!("Failed to read .gbrainignore: {}", e)))?;

    match parse_ignore_file(&content) {
        ParseResult::Valid(patterns) => {
            let patterns_json = serde_json::to_string(&patterns).unwrap();
            conn.execute(
                "UPDATE collections SET ignore_patterns = ?1, ignore_parse_errors = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                [&patterns_json, &collection_id.to_string()],
            )
            .map_err(|e| ReloadError::DbError(e.to_string()))?;
            Ok(())
        }
        ParseResult::Invalid(errors) => {
            let errors_json = serde_json::to_string(&errors).unwrap();
            conn.execute(
                "UPDATE collections SET ignore_parse_errors = ?1, updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?2",
                [&errors_json, &collection_id.to_string()],
            )
            .map_err(|e| ReloadError::DbError(e.to_string()))?;
            Err(ReloadError::ParseErrors(errors))
        }
    }
}

#[derive(Debug)]
pub enum ReloadError {
    DbError(String),
    IoError(String),
    ParseErrors(Vec<IgnoreParseError>),
    FileStablyAbsent(IgnoreParseError),
}

impl std::fmt::Display for ReloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DbError(msg) => write!(f, "Database error: {}", msg),
            Self::IoError(msg) => write!(f, "I/O error: {}", msg),
            Self::ParseErrors(errors) => {
                write!(f, "Parse errors in .gbrainignore ({} lines):", errors.len())?;
                for e in errors {
                    write!(f, "\n  Line {}: {}", e.line, e.message)?;
                }
                Ok(())
            }
            Self::FileStablyAbsent(err) => write!(f, "{}", err.message),
        }
    }
}

impl std::error::Error for ReloadError {}

// ── Build GlobSet ─────────────────────────────────────────────

/// Build a `GlobSet` from both built-in defaults and user patterns.
///
/// User patterns come from the DB mirror (`collections.ignore_patterns`).
/// If the mirror is NULL or invalid JSON, only built-in defaults are used.
pub fn build_globset(conn: &Connection, collection_id: i64) -> Result<GlobSet, String> {
    let user_patterns: Option<String> = conn
        .query_row(
            "SELECT ignore_patterns FROM collections WHERE id = ?1",
            [collection_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to query ignore_patterns: {}", e))?;

    build_globset_from_patterns(user_patterns.as_deref())
}

/// Build a `GlobSet` from built-ins plus a serialized ignore mirror payload.
///
/// This is used by the reconciler after `.gbrainignore` has been atomically
/// reloaded into `collections.ignore_patterns`.
pub fn build_globset_from_patterns(user_patterns_json: Option<&str>) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();

    // Add built-in defaults
    for pattern in builtin_patterns() {
        let glob = Glob::new(pattern).expect("Built-in pattern must be valid");
        builder.add(glob);
    }

    // Add user patterns from mirror
    if let Some(json) = user_patterns_json {
        if let Ok(patterns) = serde_json::from_str::<Vec<String>>(json) {
            for pattern in patterns {
                // Already validated during atomic parse, but be defensive
                if let Ok(glob) = Glob::new(&pattern) {
                    builder.add(glob);
                }
            }
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build GlobSet: {}", e))
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn open_collection_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../schema.sql")).unwrap();
        conn
    }

    fn insert_collection_with_mirror(
        conn: &rusqlite::Connection,
        root_path: &Path,
        ignore_patterns: Option<&str>,
        ignore_parse_errors: Option<&str>,
    ) {
        conn.execute(
            "INSERT INTO collections (name, root_path, ignore_patterns, ignore_parse_errors) VALUES ('test', ?1, ?2, ?3)",
            rusqlite::params![root_path.to_str().unwrap(), ignore_patterns, ignore_parse_errors],
        )
        .unwrap();
    }

    fn fetch_ignore_mirror(conn: &rusqlite::Connection) -> Option<String> {
        conn.query_row(
            "SELECT ignore_patterns FROM collections WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn fetch_ignore_errors(conn: &rusqlite::Connection) -> Option<String> {
        conn.query_row(
            "SELECT ignore_parse_errors FROM collections WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn builtin_patterns_are_valid() {
        for pattern in builtin_patterns() {
            assert!(
                Glob::new(pattern).is_ok(),
                "Invalid builtin pattern: {}",
                pattern
            );
        }
    }

    #[test]
    fn parse_valid_file() {
        let content = "*.tmp\n# comment\n\n*.bak\n";
        match parse_ignore_file(content) {
            ParseResult::Valid(patterns) => {
                assert_eq!(patterns, vec!["*.tmp", "*.bak"]);
            }
            ParseResult::Invalid(_) => panic!("Expected valid parse"),
        }
    }

    #[test]
    fn parse_invalid_file_reports_errors() {
        let content = "*.tmp\n[invalid\n*.bak\n";
        match parse_ignore_file(content) {
            ParseResult::Valid(_) => panic!("Expected invalid parse"),
            ParseResult::Invalid(errors) => {
                assert_eq!(errors.len(), 1);
                assert_eq!(errors[0].line, 2);
                assert_eq!(errors[0].code, "parse_error");
            }
        }
    }

    #[test]
    fn parse_invalid_file_preserves_raw_lines_and_reports_every_error() {
        let content = "[invalid\n[still-bad\n";
        match parse_ignore_file(content) {
            ParseResult::Valid(_) => panic!("Expected invalid parse"),
            ParseResult::Invalid(errors) => {
                assert_eq!(errors.len(), 2);
                assert_eq!(errors[0].line, 1);
                assert_eq!(errors[0].raw, "[invalid");
                assert_eq!(errors[0].code, "parse_error");
                assert!(errors[0].message.starts_with("Invalid glob pattern:"));
                assert_eq!(errors[1].line, 2);
                assert_eq!(errors[1].raw, "[still-bad");
                assert_eq!(errors[1].code, "parse_error");
                assert!(errors[1].message.starts_with("Invalid glob pattern:"));
            }
        }
    }

    #[test]
    fn parse_empty_file_is_valid() {
        let content = "# only comments\n\n";
        match parse_ignore_file(content) {
            ParseResult::Valid(patterns) => {
                assert!(patterns.is_empty());
            }
            ParseResult::Invalid(_) => panic!("Expected valid parse"),
        }
    }

    #[test]
    fn reload_patterns_absent_file_no_prior_mirror() {
        let conn = open_collection_db();

        let temp_dir = tempfile::tempdir().unwrap();
        insert_collection_with_mirror(&conn, temp_dir.path(), None, None);
        let result = reload_patterns(&conn, 1, temp_dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn reload_patterns_valid_file_updates_mirror() {
        let conn = open_collection_db();

        let temp_dir = tempfile::tempdir().unwrap();
        let ignore_file = temp_dir.path().join(".gbrainignore");
        fs::write(&ignore_file, "*.tmp\n*.bak\n").unwrap();

        insert_collection_with_mirror(&conn, temp_dir.path(), None, None);

        let result = reload_patterns(&conn, 1, temp_dir.path());
        assert!(result.is_ok());

        let mirror = fetch_ignore_mirror(&conn);
        assert!(mirror.is_some());
        let patterns: Vec<String> = serde_json::from_str(&mirror.unwrap()).unwrap();
        assert_eq!(patterns, vec!["*.tmp", "*.bak"]);
    }

    #[test]
    fn reload_patterns_invalid_file_records_errors() {
        let conn = open_collection_db();

        let temp_dir = tempfile::tempdir().unwrap();
        let ignore_file = temp_dir.path().join(".gbrainignore");
        fs::write(&ignore_file, "[invalid\n").unwrap();

        insert_collection_with_mirror(
            &conn,
            temp_dir.path(),
            Some(&serde_json::to_string(&vec!["private/**"]).unwrap()),
            None,
        );

        let result = reload_patterns(&conn, 1, temp_dir.path());
        assert!(result.is_err());

        let errors_json = fetch_ignore_errors(&conn);
        assert!(errors_json.is_some());
        let errors: Vec<IgnoreParseError> = serde_json::from_str(&errors_json.unwrap()).unwrap();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "parse_error");
        let mirror: Vec<String> =
            serde_json::from_str(&fetch_ignore_mirror(&conn).unwrap()).unwrap();
        assert_eq!(mirror, vec!["private/**"]);
    }

    #[test]
    fn reload_patterns_absent_file_with_prior_mirror_returns_canonical_error_shape() {
        let conn = open_collection_db();
        let temp_dir = tempfile::tempdir().unwrap();
        let mirror_json = serde_json::to_string(&vec!["private/**"]).unwrap();
        insert_collection_with_mirror(&conn, temp_dir.path(), Some(&mirror_json), None);

        let error = reload_patterns(&conn, 1, temp_dir.path()).unwrap_err();
        let ReloadError::FileStablyAbsent(shape) = error else {
            panic!("expected stable absence error");
        };

        assert_eq!(shape.code, "file_stably_absent_but_clear_not_confirmed");
        assert_eq!(shape.line, 0);
        assert_eq!(shape.raw, "");
        assert!(shape.message.contains("ignore clear"));

        let mirror: Vec<String> =
            serde_json::from_str(&fetch_ignore_mirror(&conn).unwrap()).unwrap();
        assert_eq!(mirror, vec!["private/**"]);
        let stored_errors: Vec<IgnoreParseError> =
            serde_json::from_str(&fetch_ignore_errors(&conn).unwrap()).unwrap();
        assert_eq!(stored_errors, vec![shape]);
    }

    #[test]
    fn reload_patterns_valid_file_clears_prior_parse_errors() {
        let conn = open_collection_db();
        let temp_dir = tempfile::tempdir().unwrap();
        let ignore_file = temp_dir.path().join(".gbrainignore");
        fs::write(&ignore_file, "archive/**\n").unwrap();
        let prior_errors = serde_json::to_string(&vec![IgnoreParseError {
            code: "parse_error".to_owned(),
            line: 2,
            raw: "**]".to_owned(),
            message: "Invalid glob pattern: bad".to_owned(),
        }])
        .unwrap();
        insert_collection_with_mirror(&conn, temp_dir.path(), None, Some(&prior_errors));

        reload_patterns(&conn, 1, temp_dir.path()).unwrap();

        let mirror: Vec<String> =
            serde_json::from_str(&fetch_ignore_mirror(&conn).unwrap()).unwrap();
        assert_eq!(mirror, vec!["archive/**"]);
        assert!(fetch_ignore_errors(&conn).is_none());
    }

    #[test]
    fn build_globset_includes_builtins() {
        let conn = open_collection_db();
        insert_collection_with_mirror(&conn, Path::new("/test"), None, None);

        let globset = build_globset(&conn, 1).unwrap();
        assert!(globset.is_match(".obsidian/config"));
        assert!(globset.is_match(".git/HEAD"));
        assert!(globset.is_match("node_modules/pkg/index.js"));
    }

    #[test]
    fn build_globset_includes_user_patterns() {
        let conn = open_collection_db();
        insert_collection_with_mirror(
            &conn,
            Path::new("/test"),
            Some(&serde_json::to_string(&vec!["*.tmp", "*.bak"]).unwrap()),
            None,
        );

        let globset = build_globset(&conn, 1).unwrap();
        assert!(globset.is_match("test.tmp"));
        assert!(globset.is_match("file.bak"));
        assert!(globset.is_match(".git/config")); // builtin still applies
    }

    #[test]
    fn build_globset_from_patterns_includes_builtins_and_user_patterns() {
        let globset = build_globset_from_patterns(Some(r#"["private/**","*.bak"]"#)).unwrap();

        assert!(globset.is_match("private/plan.md"));
        assert!(globset.is_match("notes/archive.bak"));
        assert!(globset.is_match(".obsidian/workspace.json"));
    }
}
