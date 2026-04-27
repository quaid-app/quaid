use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;
use rusqlite::Connection;
use serde::Serialize;
use thiserror::Error;

use crate::core::types::Page;

const CONTRADICTION_TYPE: &str = "assertion_conflict";
const OPEN_RANGE_START: &str = "";
const OPEN_RANGE_END: &str = "9999-12-31T23:59:59Z";
const MIN_OBJECT_LEN: usize = 6;
const SUPPORTED_FRONTMATTER_PREDICATES: [&str; 3] = ["is_a", "works_at", "founded"];

/// A heuristic subject-predicate-object triple extracted from page content.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Triple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

/// A stored contradiction row surfaced by `quaid check`.
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
    let page_id = resolve_page_id_for_page(conn, page)?;
    let subject = assertion_subject(page);
    let mut extracted = extract_from_frontmatter(subject, &page.frontmatter);
    let mut seen: HashSet<Triple> = extracted
        .iter()
        .map(|assertion| assertion.triple.clone())
        .collect();

    for assertion in extract_from_content(&page.compiled_truth) {
        if seen.insert(assertion.triple.clone()) {
            extracted.push(assertion);
        }
    }

    conn.execute(
        "DELETE FROM assertions WHERE page_id = ?1 AND asserted_by = 'import'",
        [page_id],
    )?;

    for assertion in &extracted {
        conn.execute(
            "INSERT INTO assertions (
                page_id, subject, predicate, object, valid_from, valid_until,
                confidence, asserted_by, source_ref, evidence_text
            ) VALUES (?1, ?2, ?3, ?4, NULL, NULL, 0.8, 'import', '', ?5)",
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

fn assertion_subject(page: &Page) -> &str {
    let title = page.title.trim();
    if title.is_empty() {
        &page.slug
    } else {
        title
    }
}

/// Detect contradictions for the requested page and insert any newly discovered rows.
#[allow(dead_code)]
pub fn check_assertions(
    slug: &str,
    conn: &Connection,
) -> Result<Vec<Contradiction>, AssertionError> {
    let root_page_id = resolve_page_id(conn, slug)?;
    check_assertions_for_page_id(root_page_id, conn)
}

pub fn check_assertions_for_page_id(
    root_page_id: i64,
    conn: &Connection,
) -> Result<Vec<Contradiction>, AssertionError> {
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

fn resolve_page_id_for_page(conn: &Connection, page: &Page) -> Result<i64, AssertionError> {
    match conn.query_row(
        "SELECT id FROM pages WHERE uuid = ?1",
        [&page.uuid],
        |row| row.get(0),
    ) {
        Ok(page_id) => Ok(page_id),
        Err(rusqlite::Error::QueryReturnedNoRows) => resolve_page_id(conn, &page.slug),
        Err(other) => Err(AssertionError::Sqlite(other)),
    }
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
           AND description = ?4
           AND resolved_at IS NULL",
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

fn extract_assertions_section(content: &str) -> &str {
    let mut offset = 0;
    let mut section_start = None;
    let mut in_fenced_code = false;

    for line in content.split_inclusive('\n') {
        let raw_line = line.trim_end_matches(['\r', '\n']);

        if is_fenced_code_boundary(raw_line) {
            in_fenced_code = !in_fenced_code;
            offset += line.len();
            continue;
        }

        if in_fenced_code {
            offset += line.len();
            continue;
        }

        if let Some(start) = section_start {
            if is_level_two_heading(raw_line) {
                return &content[start..offset];
            }
        } else if is_assertions_heading(raw_line) {
            section_start = Some(offset + line.len());
        }

        offset += line.len();
    }

    match section_start {
        Some(start) => &content[start..],
        None => "",
    }
}

fn is_assertions_heading(line: &str) -> bool {
    let Some(line) = markdown_heading_candidate(line) else {
        return false;
    };

    matches!(
        line.strip_prefix("##"),
        Some(rest)
            if !rest.starts_with('#')
                && normalize_heading_text(rest).eq_ignore_ascii_case("assertions")
    )
}

fn is_level_two_heading(line: &str) -> bool {
    let Some(line) = markdown_heading_candidate(line) else {
        return false;
    };

    matches!(line.strip_prefix("##"), Some(rest) if !rest.starts_with('#'))
}

fn is_fenced_code_boundary(line: &str) -> bool {
    let Some(line) = markdown_heading_candidate(line) else {
        return false;
    };
    let trimmed = line.trim_end();

    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn markdown_heading_candidate(line: &str) -> Option<&str> {
    let mut leading_spaces = 0;

    for (idx, ch) in line.char_indices() {
        match ch {
            ' ' if leading_spaces < 3 => leading_spaces += 1,
            ' ' | '\t' => return None,
            _ => return Some(&line[idx..]),
        }
    }

    Some("")
}

fn normalize_heading_text(line: &str) -> &str {
    line.trim().trim_end_matches('#').trim_end()
}

fn extract_from_frontmatter(
    subject: &str,
    frontmatter: &HashMap<String, String>,
) -> Vec<ExtractedAssertion> {
    let subject = normalize_evidence(subject);

    SUPPORTED_FRONTMATTER_PREDICATES
        .iter()
        .filter_map(|predicate| {
            let object = normalize_evidence(frontmatter.get(*predicate)?);
            if !is_valid_object(&object) {
                return None;
            }

            Some(ExtractedAssertion {
                triple: Triple {
                    subject: subject.clone(),
                    predicate: (*predicate).to_string(),
                    object,
                },
                evidence_text: "frontmatter".to_string(),
            })
        })
        .collect()
}

fn extract_from_content(content: &str) -> Vec<ExtractedAssertion> {
    let scoped_content = extract_assertions_section(content);
    if scoped_content.trim().is_empty() {
        return Vec::new();
    }

    let mut extracted = Vec::new();
    let mut seen = HashSet::new();

    // Pattern 1: "Alice works at Acme Corp" -> (Alice, works_at, Acme Corp)
    collect_pattern_matches(
        scoped_content,
        works_at_regex(),
        "works_at",
        &mut seen,
        &mut extracted,
    );
    // Pattern 2: "Alice is a founder" -> (Alice, is_a, founder)
    collect_pattern_matches(
        scoped_content,
        is_a_regex(),
        "is_a",
        &mut seen,
        &mut extracted,
    );
    // Pattern 3: "Alice founded Brain Co" -> (Alice, founded, Brain Co)
    collect_pattern_matches(
        scoped_content,
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

        if !is_valid_object(&triple.object) {
            continue;
        }

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

fn is_valid_object(object: &str) -> bool {
    object.trim().len() >= MIN_OBJECT_LEN
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
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::get::get_page;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").unwrap()
    }

    fn insert_page(conn: &Connection, slug: &str, truth: &str) {
        insert_page_with_frontmatter(conn, slug, slug, truth, "{}");
    }

    fn test_uuid(slug: &str) -> String {
        let mut hex = String::new();
        for byte in slug.as_bytes() {
            hex.push_str(&format!("{byte:02x}"));
            if hex.len() >= 32 {
                break;
            }
        }
        while hex.len() < 32 {
            hex.push('0');
        }

        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    }

    fn insert_page_with_frontmatter(
        conn: &Connection,
        slug: &str,
        title: &str,
        truth: &str,
        frontmatter: &str,
    ) {
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                                 frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'person', ?3, '', ?4, '', ?5, 'people', '', 1)",
            rusqlite::params![slug, test_uuid(slug), title, truth, frontmatter],
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

    #[cfg(test)]
    mod extract_assertions {
        use super::*;

        #[test]
        fn inserts_expected_triples_for_supported_patterns() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "Alice biography.\n\n## Assertions\nAlice works at Acme Corp.\nAlice is a founder.\nAlice founded Brain Co.\n",
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
                        "import".to_string(),
                    ),
                    (
                        "Alice".to_string(),
                        "is_a".to_string(),
                        "founder".to_string(),
                        0.8,
                        "import".to_string(),
                    ),
                    (
                        "Alice".to_string(),
                        "works_at".to_string(),
                        "Acme Corp".to_string(),
                        0.8,
                        "import".to_string(),
                    ),
                ]
            );
        }

        #[test]
        fn duplicate_bare_slugs_across_collections_use_page_uuid_for_import_assertions() {
            let conn = open_test_db();
            conn.execute(
                "INSERT INTO collections (name, root_path, state, writable, is_write_target)
                 VALUES ('memory', 'C:\\vaults\\memory', 'active', 1, 0)",
                [],
            )
            .unwrap();
            let memory_id = conn.last_insert_rowid();

            conn.execute(
                "INSERT INTO pages (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
                 VALUES (1, 'people/alice', '11111111-1111-7111-8111-111111111111', 'person', 'Default Alice', '', '## Assertions\nAlice works at Acme Corp.\n', '', '{}', 'people', '', 1)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO pages (collection_id, slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version)
                 VALUES (?1, 'people/alice', '22222222-2222-7222-8222-222222222222', 'person', 'Memory Alice', '', '## Assertions\nAlice works at Beta Corp.\n', '', '{}', 'people', '', 1)",
                [memory_id],
            )
            .unwrap();
            let memory_page_id: i64 = conn
                .query_row(
                    "SELECT id FROM pages WHERE collection_id = ?1 AND slug = 'people/alice'",
                    [memory_id],
                    |row| row.get(0),
                )
                .unwrap();

            let page =
                crate::commands::get::get_page_by_key(&conn, memory_id, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();
            let rows: Vec<(i64, String)> = conn
                .prepare("SELECT page_id, object FROM assertions ORDER BY object")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(inserted, 1);
            assert_eq!(rows, vec![(memory_page_id, "Beta Corp".to_string())]);
        }

        #[test]
        fn reindexing_replaces_prior_import_triples() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "## Assertions\nAlice works at Acme Corp.\n",
            );
            let first_page = get_page(&conn, "people/alice").unwrap();
            extract_assertions(&first_page, &conn).unwrap();

            update_page_truth(
                &conn,
                "people/alice",
                "## Assertions\nAlice founded Brain Co.\n",
            );
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
                "Alice works at Acme Corp. This is general prose outside a structured section.",
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
        fn duplicate_matches_in_content_insert_one_row() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "## Assertions\nAlice works at Acme Corp.\nAlice works at Acme Corp.\n",
            );
            let page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();

            assert_eq!(inserted, 1);
        }

        #[test]
        fn reindexing_preserves_manual_assertions() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "## Assertions\nAlice works at Acme Corp.\n",
            );
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

            update_page_truth(
                &conn,
                "people/alice",
                "## Assertions\nAlice founded Brain Co.\n",
            );
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
                        "import".to_string(),
                    ),
                    (
                        "employer".to_string(),
                        "Manual Corp".to_string(),
                        "manual".to_string(),
                    ),
                ]
            );
        }

        #[test]
        fn extracts_frontmatter_assertions() {
            let conn = open_test_db();
            insert_page_with_frontmatter(
                &conn,
                "people/alice",
                "Alice",
                "Alice biography.",
                r#"{"is_a":"researcher"}"#,
            );
            let page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();
            let rows: Vec<(String, String, String, String)> = conn
                .prepare(
                    "SELECT subject, predicate, object, evidence_text
                     FROM assertions
                     ORDER BY predicate, object",
                )
                .unwrap()
                .query_map([], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(inserted, 1);
            assert_eq!(
                rows,
                vec![(
                    "Alice".to_string(),
                    "is_a".to_string(),
                    "researcher".to_string(),
                    "frontmatter".to_string(),
                )]
            );
        }

        #[test]
        fn short_frontmatter_object_is_discarded() {
            let conn = open_test_db();
            insert_page_with_frontmatter(
                &conn,
                "people/alice",
                "Alice",
                "## Assertions\nAlice works at Acme Corp.\n",
                r#"{"is_a":"it"}"#,
            );
            let page = get_page(&conn, "people/alice").unwrap();

            let inserted = extract_assertions(&page, &conn).unwrap();
            let rows: Vec<(String, String)> = conn
                .prepare("SELECT predicate, object FROM assertions ORDER BY predicate, object")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();

            assert_eq!(inserted, 1);
            assert_eq!(
                rows,
                vec![("works_at".to_string(), "Acme Corp".to_string())]
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
            insert_page(
                &conn,
                "people/alice",
                "## Assertions\nAlice works at Acme Corp.\n",
            );
            insert_page(
                &conn,
                "sources/alice-profile",
                "## Assertions\nAlice works at Beta Corp.\n",
            );
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
        fn resolved_conflict_is_redetected() {
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

            assert_eq!(
                contradictions.len(),
                1,
                "resolved contradiction should be re-detected"
            );
            assert_eq!(
                contradiction_count(&conn),
                2,
                "old resolved + new unresolved"
            );
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

        #[test]
        fn clean_page_returns_no_contradictions() {
            let conn = open_test_db();
            insert_page(
                &conn,
                "people/alice",
                "## Assertions\nAlice works at Acme Corp.\n",
            );
            let page = get_page(&conn, "people/alice").unwrap();
            extract_assertions(&page, &conn).unwrap();

            let contradictions = check_assertions("people/alice", &conn).unwrap();

            assert!(contradictions.is_empty());
        }

        #[test]
        fn unknown_slug_returns_page_not_found() {
            let conn = open_test_db();

            let error = check_assertions("people/ghost", &conn).unwrap_err();

            assert!(matches!(error, AssertionError::PageNotFound { .. }));
        }
    }
}
