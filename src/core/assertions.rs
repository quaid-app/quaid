use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

use crate::core::types::Page;

const CONTRADICTION_TYPE: &str = "assertion_conflict";
const OPEN_RANGE_START: &str = "";
const OPEN_RANGE_END: &str = "9999-12-31T23:59:59Z";

/// A heuristic subject-predicate-object triple extracted from page content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Triple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// A stored contradiction row surfaced by `gbrain check`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Contradiction {
    pub page_slug: String,
    pub other_page_slug: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub description: String,
    pub detected_at: String,
}

#[derive(Debug, Error)]
pub enum AssertionError {
    #[error("page not found: {slug}")]
    PageNotFound { slug: String },

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[derive(Debug, Clone)]
struct ExtractedAssertion {
    triple: Triple,
    evidence_text: String,
}

#[derive(Debug, Clone)]
struct AssertionRow {
    page_id: i64,
    page_slug: String,
    subject: String,
    predicate: String,
    object: String,
    valid_from: Option<String>,
    valid_until: Option<String>,
}

/// Replace heuristic assertions for a page and return the number of inserted triples.
pub fn extract_assertions(page: &Page, conn: &Connection) -> Result<usize, AssertionError> {
    let page_id = resolve_page_id(conn, &page.slug)?;
    let extracted = extract_from_content(&page.compiled_truth);

    conn.execute(
        "DELETE FROM assertions WHERE page_id = ?1 AND asserted_by = 'agent'",
        [page_id],
    )?;

    for assertion in &extracted {
        conn.execute(
            "INSERT INTO assertions (
                page_id, subject, predicate, object, valid_from, valid_until,
                confidence, asserted_by, source_ref, evidence_text
            ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 0.8, 'agent', '', ?5)",
            rusqlite::params![
                page_id,
                assertion.triple.subject,
                assertion.triple.predicate,
                assertion.triple.object,
                assertion.evidence_text,
            ],
        )?;
    }

    Ok(extracted.len())
}

/// Detect contradictions for the requested page and insert any newly discovered rows.
pub fn check_assertions(
    slug: &str,
    conn: &Connection,
) -> Result<Vec<Contradiction>, AssertionError> {
    let root_page_id = resolve_page_id(conn, slug)?;
    let subjects = load_subjects_for_page(conn, root_page_id)?;
    let mut contradictions = Vec::new();

    for subject in subjects {
        let assertions = load_assertions_for_subject(conn, &subject)?;

        for left_index in 0..assertions.len() {
            for right_index in (left_index + 1)..assertions.len() {
                let left = &assertions[left_index];
                let right = &assertions[right_index];

                if left.predicate != right.predicate
                    || left.object == right.object
                    || !validity_windows_overlap(left, right)
                    || (left.page_id != root_page_id && right.page_id != root_page_id)
                {
                    continue;
                }

                let (page_row, other_row) = canonical_pair(left, right);
                let description = conflict_description(
                    &left.subject,
                    &left.predicate,
                    &left.object,
                    &right.object,
                );

                if contradiction_exists(conn, page_row.page_id, other_row.page_id, &description)? {
                    continue;
                }

                conn.execute(
                    "INSERT INTO contradictions (page_id, other_page_id, type, description)
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![
                        page_row.page_id,
                        other_row.page_id,
                        CONTRADICTION_TYPE,
                        description,
                    ],
                )?;

                contradictions.push(load_contradiction(conn, conn.last_insert_rowid())?);
            }
        }
    }

    contradictions.sort_by(|left, right| {
        (
            left.page_slug.as_str(),
            left.other_page_slug.as_str(),
            left.description.as_str(),
        )
            .cmp(&(
                right.page_slug.as_str(),
                right.other_page_slug.as_str(),
                right.description.as_str(),
            ))
    });

    Ok(contradictions)
}

fn resolve_page_id(conn: &Connection, slug: &str) -> Result<i64, AssertionError> {
    conn.query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
        row.get(0)
    })
    .map_err(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => AssertionError::PageNotFound {
            slug: slug.to_string(),
        },
        other => AssertionError::Sqlite(other),
    })
}

fn load_subjects_for_page(conn: &Connection, page_id: i64) -> Result<Vec<String>, AssertionError> {
    let mut statement = conn
        .prepare("SELECT DISTINCT subject FROM assertions WHERE page_id = ?1 ORDER BY subject")?;
    let subjects = statement
        .query_map([page_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(subjects)
}

fn load_assertions_for_subject(
    conn: &Connection,
    subject: &str,
) -> Result<Vec<AssertionRow>, AssertionError> {
    let mut statement = conn.prepare(
        "SELECT a.page_id, p.slug, a.subject, a.predicate, a.object, a.valid_from, a.valid_until
         FROM assertions a
         JOIN pages p ON p.id = a.page_id
         WHERE a.subject = ?1
         ORDER BY a.predicate, a.object, p.slug",
    )?;
    let assertions = statement
        .query_map([subject], |row| {
            Ok(AssertionRow {
                page_id: row.get(0)?,
                page_slug: row.get(1)?,
                subject: row.get(2)?,
                predicate: row.get(3)?,
                object: row.get(4)?,
                valid_from: row.get(5)?,
                valid_until: row.get(6)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(assertions)
}

fn contradiction_exists(
    conn: &Connection,
    page_id: i64,
    other_page_id: i64,
    description: &str,
) -> Result<bool, AssertionError> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM contradictions
         WHERE page_id = ?1
           AND other_page_id = ?2
           AND type = ?3
           AND description = ?4",
        rusqlite::params![page_id, other_page_id, CONTRADICTION_TYPE, description],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

fn load_contradiction(
    conn: &Connection,
    contradiction_id: i64,
) -> Result<Contradiction, AssertionError> {
    conn.query_row(
        "SELECT p.slug,
                COALESCE(other.slug, p.slug),
                c.type,
                c.description,
                c.detected_at
         FROM contradictions c
         JOIN pages p ON p.id = c.page_id
         LEFT JOIN pages other ON other.id = c.other_page_id
         WHERE c.id = ?1",
        [contradiction_id],
        |row| {
            Ok(Contradiction {
                page_slug: row.get(0)?,
                other_page_slug: row.get(1)?,
                r#type: row.get(2)?,
                description: row.get(3)?,
                detected_at: row.get(4)?,
            })
        },
    )
    .map_err(AssertionError::from)
}

fn extract_from_content(content: &str) -> Vec<ExtractedAssertion> {
    let mut extracted = Vec::new();
    let mut seen = HashSet::new();

    // Pattern 1: "Alice works at Acme Corp" -> (Alice, works_at, Acme Corp)
    collect_pattern_matches(
        content,
        works_at_regex(),
        "works_at",
        &mut seen,
        &mut extracted,
    );
    // Pattern 2: "Alice is a founder" -> (Alice, is_a, founder)
    collect_pattern_matches(content, is_a_regex(), "is_a", &mut seen, &mut extracted);
    // Pattern 3: "Alice founded Brain Co" -> (Alice, founded, Brain Co)
    collect_pattern_matches(
        content,
        founded_regex(),
        "founded",
        &mut seen,
        &mut extracted,
    );

    extracted
}

fn collect_pattern_matches(
    content: &str,
    regex: &Regex,
    predicate: &str,
    seen: &mut HashSet<Triple>,
    extracted: &mut Vec<ExtractedAssertion>,
) {
    for captures in regex.captures_iter(content) {
        let triple = Triple {
            subject: normalize_capture(captures.name("subject")),
            predicate: predicate.to_string(),
            object: normalize_capture(captures.name("object")),
        };

        if seen.insert(triple.clone()) {
            extracted.push(ExtractedAssertion {
                triple,
                evidence_text: normalize_evidence(
                    captures.get(0).map(|m| m.as_str()).unwrap_or_default(),
                ),
            });
        }
    }
}

fn normalize_capture(capture: Option<regex::Match<'_>>) -> String {
    capture
        .map(|value| normalize_evidence(value.as_str()))
        .unwrap_or_default()
}

fn normalize_evidence(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(['.', '!', '?', ';', ':'])
        .trim()
        .to_string()
}

fn validity_windows_overlap(left: &AssertionRow, right: &AssertionRow) -> bool {
    let left_start = left.valid_from.as_deref().unwrap_or(OPEN_RANGE_START);
    let left_end = left.valid_until.as_deref().unwrap_or(OPEN_RANGE_END);
    let right_start = right.valid_from.as_deref().unwrap_or(OPEN_RANGE_START);
    let right_end = right.valid_until.as_deref().unwrap_or(OPEN_RANGE_END);

    left_start <= right_end && right_start <= left_end
}

fn canonical_pair<'a>(
    left: &'a AssertionRow,
    right: &'a AssertionRow,
) -> (&'a AssertionRow, &'a AssertionRow) {
    if (left.page_id, left.page_slug.as_str(), left.object.as_str())
        <= (
            right.page_id,
            right.page_slug.as_str(),
            right.object.as_str(),
        )
    {
        (left, right)
    } else {
        (right, left)
    }
}

fn conflict_description(
    subject: &str,
    predicate: &str,
    left_object: &str,
    right_object: &str,
) -> String {
    let (first_object, second_object) = if left_object <= right_object {
        (left_object, right_object)
    } else {
        (right_object, left_object)
    };

    format!("{subject} has conflicting {predicate} assertions: {first_object} vs {second_object}")
}

fn works_at_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?m)\b(?P<subject>[A-Z][A-Za-z0-9'&-]*(?: [A-Z][A-Za-z0-9'&-]*)*) works at (?P<object>[A-Z][A-Za-z0-9'&-]*(?: [A-Z][A-Za-z0-9'&-]*)*)",
        )
        .expect("valid works-at regex")
    })
}

fn is_a_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?m)\b(?P<subject>[A-Z][A-Za-z0-9'&-]*(?: [A-Z][A-Za-z0-9'&-]*)*) is an? (?P<object>[A-Za-z][A-Za-z0-9'&-]*(?: [A-Za-z][A-Za-z0-9'&-]*)*)",
        )
        .expect("valid is-a regex")
    })
}

fn founded_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r"(?m)\b(?P<subject>[A-Z][A-Za-z0-9'&-]*(?: [A-Z][A-Za-z0-9'&-]*)*) founded (?P<object>[A-Z][A-Za-z0-9'&-]*(?: [A-Z][A-Za-z0-9'&-]*)*)",
        )
        .expect("valid founded regex")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::get::get_page;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
    }

    fn insert_page(conn: &Connection, slug: &str, truth: &str) {
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, \
                                frontmatter, wing, room, version) \
             VALUES (?1, 'person', ?2, '', ?3, '', '{}', 'people', '', 1)",
            rusqlite::params![slug, slug, truth],
        )
        .unwrap();
    }

    fn update_page_truth(conn: &Connection, slug: &str, truth: &str) {
        conn.execute(
            "UPDATE pages
             SET compiled_truth = ?1,
                 version = version + 1,
                 updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
                 truth_updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
             WHERE slug = ?2",
            rusqlite::params![truth, slug],
        )
        .unwrap();
    }

    fn insert_assertion(
        conn: &Connection,
        slug: &str,
        triple: Triple,
        validity: (Option<&str>, Option<&str>),
        asserted_by: &str,
    ) {
        let page_id: i64 = conn
            .query_row("SELECT id FROM pages WHERE slug = ?1", [slug], |row| {
                row.get(0)
            })
            .unwrap();

        conn.execute(
            "INSERT INTO assertions (
                page_id, subject, predicate, object, valid_from, valid_until,
                confidence, asserted_by, source_ref, evidence_text
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1.0, ?7, '', '')",
            rusqlite::params![
                page_id,
                triple.subject,
                triple.predicate,
                triple.object,
                validity.0,
                validity.1,
                asserted_by,
            ],
        )
        .unwrap();
    }

    fn contradiction_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM contradictions", [], |row| row.get(0))
            .unwrap()
    }

    mod extract_assertions {
        use super::*;

        #[test]
        fn inserts_expected_triples_for_supported_patterns() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "Alice works at Acme Corp. Alice is a founder. Alice founded Brain Co.",
            );
            let page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();

            let mut statement = conn
                .prepare(
                    "SELECT subject, predicate, object, confidence, asserted_by
                     FROM assertions
                     ORDER BY predicate, object",
                )
                .unwrap();
            let rows = statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, f64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(inserted, 3);
            assert_eq!(
                rows,
                vec![
                    (
                        "Alice".to_string(),
                        "founded".to_string(),
                        "Brain Co".to_string(),
                        0.8,
                        "agent".to_string(),
                    ),
                    (
                        "Alice".to_string(),
                        "is_a".to_string(),
                        "founder".to_string(),
                        0.8,
                        "agent".to_string(),
                    ),
                    (
                        "Alice".to_string(),
                        "works_at".to_string(),
                        "Acme Corp".to_string(),
                        0.8,
                        "agent".to_string(),
                    ),
                ]
            );
        }

        #[test]
        fn reindexing_replaces_prior_agent_triples() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
            let first_page = get_page(&conn, "people/alice").unwrap();
            extract_assertions(&first_page, &conn).unwrap();

            update_page_truth(&conn, "people/alice", "Alice founded Brain Co.");
            let second_page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&second_page, &conn).unwrap();
            let rows: Vec<(String, String)> = conn
                .prepare("SELECT predicate, object FROM assertions ORDER BY predicate, object")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(inserted, 1);
            assert_eq!(rows, vec![("founded".to_string(), "Brain Co".to_string())]);
        }

        #[test]
        fn page_with_no_supported_patterns_inserts_zero_rows() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "This page is narrative prose without any deterministic triples.",
            );
            let page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();
            let row_count: i64 = conn
                .query_row("SELECT COUNT(*) FROM assertions", [], |row| row.get(0))
                .unwrap();

            assert_eq!(inserted, 0);
            assert_eq!(row_count, 0);
        }

        #[test]
        fn reindexing_preserves_manual_assertions() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
            let page = get_page(&conn, "people/alice").unwrap();
            extract_assertions(&page, &conn).unwrap();
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Manual Corp".to_string(),
                },
                (None, None),
                "manual",
            );

            update_page_truth(&conn, "people/alice", "Alice founded Brain Co.");
            let updated_page = get_page(&conn, "people/alice").unwrap();
            extract_assertions(&updated_page, &conn).unwrap();

            let rows: Vec<(String, String, String)> = conn
                .prepare("SELECT predicate, object, asserted_by FROM assertions ORDER BY asserted_by, predicate")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(
                rows,
                vec![
                    (
                        "founded".to_string(),
                        "Brain Co".to_string(),
                        "agent".to_string(),
                    ),
                    (
                        "employer".to_string(),
                        "Manual Corp".to_string(),
                        "manual".to_string(),
                    ),
                ]
            );
        }
    }

    mod check_assertions {
        use super::*;

        #[test]
        fn detects_same_page_conflict() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice timeline.");
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Acme".to_string(),
                },
                (None, None),
                "manual",
            );
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Beta Corp".to_string(),
                },
                (None, None),
                "manual",
            );

            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert_eq!(contradictions.len(), 1);
            assert_eq!(contradictions[0].page_slug, "people/alice");
            assert_eq!(contradictions[0].other_page_slug, "people/alice");
            assert_eq!(contradictions[0].r#type, CONTRADICTION_TYPE);
            assert!(contradictions[0].description.contains("Acme"));
            assert!(contradictions[0].description.contains("Beta Corp"));
            assert_eq!(contradiction_count(&conn), 1);
        }

        #[test]
        fn detects_cross_page_conflict() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice works at Acme Corp.");
            insert_page(&conn, "sources/alice-profile", "Alice works at Beta Corp.");
            let first_page = get_page(&conn, "people/alice").unwrap();
            let second_page = get_page(&conn, "sources/alice-profile").unwrap();
            extract_assertions(&first_page, &conn).unwrap();
            extract_assertions(&second_page, &conn).unwrap();

            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert_eq!(contradictions.len(), 1);
            assert_eq!(contradictions[0].page_slug, "people/alice");
            assert_eq!(contradictions[0].other_page_slug, "sources/alice-profile");
            assert!(contradictions[0].description.contains("Acme Corp"));
            assert!(contradictions[0].description.contains("Beta Corp"));
        }

        #[test]
        fn resolved_conflict_is_not_duplicated() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice timeline.");
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Acme".to_string(),
                },
                (None, None),
                "manual",
            );
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Beta".to_string(),
                },
                (None, None),
                "manual",
            );

            check_assertions("people/alice", &conn).unwrap();
            conn.execute(
                "UPDATE contradictions SET resolved_at = '2026-04-15T00:00:00Z'",
                [],
            )
            .unwrap();

            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert!(contradictions.is_empty());
            assert_eq!(contradiction_count(&conn), 1);
        }

        #[test]
        fn rerun_does_not_duplicate_unresolved_conflict() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice timeline.");
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Acme".to_string(),
                },
                (None, None),
                "manual",
            );
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Beta".to_string(),
                },
                (None, None),
                "manual",
            );

            check_assertions("people/alice", &conn).unwrap();
            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert!(contradictions.is_empty());
            assert_eq!(contradiction_count(&conn), 1);
        }

        #[test]
        fn skips_non_overlapping_validity_windows() {
            let conn = open_test_db();
            insert_page(&conn, "people/alice", "Alice timeline.");
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Acme".to_string(),
                },
                (Some("2020-01-01"), Some("2020-12-31")),
                "manual",
            );
            insert_assertion(
                &conn,
                "people/alice",
                Triple {
                    subject: "Alice".to_string(),
                    predicate: "employer".to_string(),
                    object: "Beta".to_string(),
                },
                (Some("2021-01-01"), Some("2021-12-31")),
                "manual",
            );

            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert!(contradictions.is_empty());
            assert_eq!(contradiction_count(&conn), 0);
        }
    }
}
