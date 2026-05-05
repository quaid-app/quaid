use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::core::file_state::{self, FileStat};
use crate::core::types::{frontmatter_get_string, frontmatter_insert_string, Frontmatter, Page};
use crate::core::{db, markdown, page_uuid, palace, raw_imports};

const ELIGIBLE_TYPES: [&str; 4] = ["decision", "preference", "fact", "action_item"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedPage {
    pub slug: String,
    pub title: String,
    pub page_type: String,
    pub summary: String,
    pub compiled_truth: String,
    pub timeline: String,
    pub frontmatter: Frontmatter,
    pub wing: String,
    pub room: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandleExtractedEditOutcome {
    Bypass,
    WhitespaceNoOp,
    Superseded {
        archived_slug: String,
        history_path: Option<PathBuf>,
    },
}

pub fn handles_page_type(page_type: &str) -> bool {
    eligible_type(page_type)
}

pub fn normalized_content_key(raw_bytes: &[u8]) -> String {
    normalize_whitespace_lossy(raw_bytes)
}

#[derive(Debug, Error)]
pub enum FileEditError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config error: {0}")]
    Db(#[from] crate::core::types::DbError),
    #[error("page UUID error: {0}")]
    PageUuid(#[from] crate::core::page_uuid::PageUuidError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "file-edit supersede requires head page `{slug}`; current successor is `{successor_slug}`"
    )]
    NonHeadTarget {
        slug: String,
        successor_slug: String,
    },
}

pub fn parse_edited_page(
    raw_bytes: &[u8],
    file_path: &Path,
    root_path: &Path,
) -> Result<EditedPage, FileEditError> {
    let raw = String::from_utf8_lossy(raw_bytes).into_owned();
    let (frontmatter, body) = markdown::parse_frontmatter(&raw);
    let (compiled_truth, timeline) = markdown::split_content(&body);
    let slug = frontmatter_get_string(&frontmatter, "slug")
        .unwrap_or_else(|| derive_slug_from_path(file_path, root_path));
    let title = frontmatter_get_string(&frontmatter, "title").unwrap_or_else(|| slug.clone());
    let page_type = frontmatter
        .get("type")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && !value.eq_ignore_ascii_case("null"))
        .map(str::to_owned)
        .or_else(|| infer_type_from_path(file_path, root_path))
        .unwrap_or_else(|| "concept".to_string());
    let wing =
        frontmatter_get_string(&frontmatter, "wing").unwrap_or_else(|| palace::derive_wing(&slug));
    Ok(EditedPage {
        summary: markdown::extract_summary(&compiled_truth),
        room: palace::derive_room(&compiled_truth),
        sha256: raw_imports::content_hash_hex(raw_bytes),
        slug,
        title,
        page_type,
        compiled_truth,
        timeline,
        frontmatter,
        wing,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn handle_extracted_edit(
    conn: &Connection,
    collection_id: i64,
    page_id: i64,
    relative_path: &Path,
    root_path: &Path,
    _stat: &FileStat,
    prior_page: &Page,
    new_page: &EditedPage,
    new_raw_bytes: &[u8],
) -> Result<HandleExtractedEditOutcome, FileEditError> {
    if !is_extracted_path(relative_path) || !eligible_type(&prior_page.page_type) {
        return Ok(HandleExtractedEditOutcome::Bypass);
    }

    if let Some(successor_id) = prior_page.superseded_by {
        let successor_slug: String = conn.query_row(
            "SELECT c.name || '::' || p.slug
             FROM pages p
             JOIN collections c ON c.id = p.collection_id
             WHERE p.id = ?1",
            [successor_id],
            |row| row.get(0),
        )?;
        return Err(FileEditError::NonHeadTarget {
            slug: prior_page.slug.clone(),
            successor_slug,
        });
    }

    let tx = conn.unchecked_transaction()?;
    let prior_raw_bytes = active_raw_bytes(&tx, page_id)?;
    if normalize_whitespace_lossy(&prior_raw_bytes) == normalize_whitespace_lossy(new_raw_bytes) {
        tx.rollback()?;
        return Ok(HandleExtractedEditOutcome::WhitespaceNoOp);
    }

    let namespace = page_namespace(&tx, page_id)?;
    let now = current_timestamp(&tx)?;
    let archived_suffix = slug_timestamp(&now);
    let archived_slug = format!("{}--archived-{archived_suffix}", prior_page.slug);
    let mut current_frontmatter = new_page.frontmatter.clone();
    frontmatter_insert_string(
        &mut current_frontmatter,
        "supersedes",
        archived_slug.clone(),
    );
    frontmatter_insert_string(&mut current_frontmatter, "corrected_via", "file_edit");
    frontmatter_insert_string(
        &mut current_frontmatter,
        "type",
        prior_page.page_type.clone(),
    );
    let current_uuid = page_uuid::resolve_page_uuid(&current_frontmatter, Some(&prior_page.uuid))?;
    let archived_uuid = page_uuid::generate_uuid_v7();
    let archived_frontmatter_json = serde_json::to_string(&prior_page.frontmatter)?;
    let current_frontmatter_json = serde_json::to_string(&current_frontmatter)?;
    let previous_predecessor_id = current_predecessor(&tx, collection_id, &namespace, page_id)?;
    let rendered_live = markdown::render_page(&Page {
        slug: new_page.slug.clone(),
        uuid: current_uuid.clone(),
        page_type: prior_page.page_type.clone(),
        superseded_by: None,
        title: new_page.title.clone(),
        summary: new_page.summary.clone(),
        compiled_truth: new_page.compiled_truth.clone(),
        timeline: new_page.timeline.clone(),
        frontmatter: current_frontmatter.clone(),
        wing: new_page.wing.clone(),
        room: new_page.room.clone(),
        version: prior_page.version + 1,
        created_at: now.clone(),
        updated_at: now.clone(),
        truth_updated_at: now.clone(),
        timeline_updated_at: now.clone(),
    });
    let live_path = root_path.join(relative_path);
    let history_path = if history_on_disk_enabled(&tx)? {
        let history_path = history_relative_path(relative_path, &prior_page.slug, &archived_suffix);
        let absolute_history_path = root_path.join(&history_path);
        if let Some(parent) = absolute_history_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&absolute_history_path, &prior_raw_bytes)?;
        Some(history_path)
    } else {
        None
    };
    let mut live_rewritten = false;
    let result = (|| -> Result<HandleExtractedEditOutcome, FileEditError> {
        if let Some(parent) = live_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&live_path, rendered_live.as_bytes())?;
        live_rewritten = true;
        let final_stat = file_state::stat_file(&live_path)?;
        let final_hash = raw_imports::content_hash_hex(rendered_live.as_bytes());

        tx.execute(
            "INSERT INTO pages
                 (collection_id, namespace, slug, uuid, type, title, summary, compiled_truth, timeline,
                  frontmatter, wing, room, superseded_by, version,
                  created_at, updated_at, truth_updated_at, timeline_updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 1, ?14, ?14, ?14, ?14)",
            params![
                collection_id,
                namespace.clone(),
                archived_slug.clone(),
                archived_uuid,
                prior_page.page_type.clone(),
                prior_page.title.clone(),
                prior_page.summary.clone(),
                prior_page.compiled_truth.clone(),
                prior_page.timeline.clone(),
                archived_frontmatter_json,
                prior_page.wing.clone(),
                prior_page.room.clone(),
                page_id,
                now.clone()
            ],
        )?;
        let archived_page_id = tx.last_insert_rowid();

        if let Some(previous_predecessor_id) = previous_predecessor_id {
            tx.execute(
                "UPDATE pages
                 SET superseded_by = ?1
                 WHERE id = ?2 AND superseded_by = ?3",
                params![archived_page_id, previous_predecessor_id, page_id],
            )?;
        }

        let updated = tx.execute(
            "UPDATE pages
             SET slug = ?1,
                 uuid = ?2,
                 type = ?3,
                 title = ?4,
                 summary = ?5,
                 compiled_truth = ?6,
                 timeline = ?7,
                 frontmatter = ?8,
                 wing = ?9,
                 room = ?10,
                 superseded_by = NULL,
                 version = version + 1,
                 quarantined_at = NULL,
                 updated_at = ?11,
                 truth_updated_at = ?11,
                 timeline_updated_at = ?11
             WHERE id = ?12 AND superseded_by IS NULL",
            params![
                new_page.slug.clone(),
                current_uuid,
                prior_page.page_type.clone(),
                new_page.title.clone(),
                new_page.summary.clone(),
                new_page.compiled_truth.clone(),
                new_page.timeline.clone(),
                current_frontmatter_json,
                new_page.wing.clone(),
                new_page.room.clone(),
                now.clone(),
                page_id
            ],
        )?;
        if updated == 0 {
            return Err(FileEditError::NonHeadTarget {
                slug: prior_page.slug.clone(),
                successor_slug: "unknown".to_string(),
            });
        }

        raw_imports::rotate_active_raw_import(
            &tx,
            page_id,
            &live_path.to_string_lossy(),
            rendered_live.as_bytes(),
        )?;

        raw_imports::enqueue_embedding_job(&tx, page_id)?;
        raw_imports::enqueue_embedding_job(&tx, archived_page_id)?;
        file_state::upsert_file_state(
            &tx,
            collection_id,
            &path_to_string(relative_path),
            page_id,
            &final_stat,
            &final_hash,
        )?;
        tx.commit()?;

        Ok(HandleExtractedEditOutcome::Superseded {
            archived_slug,
            history_path: history_path.clone(),
        })
    })();

    if result.is_err() {
        if live_rewritten {
            let _ = fs::write(&live_path, new_raw_bytes);
        }
        if let Some(history_path) = history_path.as_ref() {
            let _ = fs::remove_file(root_path.join(history_path));
        }
    }

    result
}

pub fn is_extracted_path(relative_path: &Path) -> bool {
    let parts = path_parts(relative_path);
    matches!(parts.as_slice(), ["extracted", ..] | [_, "extracted", ..])
        && !is_history_sidecar_path(relative_path)
}

pub fn is_history_sidecar_path(relative_path: &Path) -> bool {
    matches!(
        path_parts(relative_path).as_slice(),
        ["extracted", "_history", ..] | [_, "extracted", "_history", ..]
    )
}

pub fn is_extracted_whitespace_noop(
    conn: &Connection,
    collection_id: i64,
    root_path: &Path,
    relative_path: &Path,
    page_id: i64,
) -> Result<bool, FileEditError> {
    if !is_extracted_path(relative_path) {
        return Ok(false);
    }
    let page_type = conn.query_row(
        "SELECT type
         FROM pages
         WHERE id = ?1 AND collection_id = ?2",
        params![page_id, collection_id],
        |row| row.get::<_, String>(0),
    )?;
    if !handles_page_type(&page_type) {
        return Ok(false);
    }
    let disk_bytes = fs::read(root_path.join(relative_path))?;
    Ok(normalized_content_key(&active_raw_bytes(conn, page_id)?)
        == normalized_content_key(&disk_bytes))
}

fn active_raw_bytes(conn: &Connection, page_id: i64) -> Result<Vec<u8>, FileEditError> {
    conn.query_row(
        "SELECT raw_bytes
         FROM raw_imports
         WHERE page_id = ?1 AND is_active = 1
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
        [page_id],
        |row| row.get(0),
    )
    .map_err(FileEditError::from)
}

fn current_predecessor(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    page_id: i64,
) -> Result<Option<i64>, FileEditError> {
    conn.query_row(
        "SELECT id
         FROM pages
         WHERE collection_id = ?1 AND namespace = ?2 AND superseded_by = ?3
         LIMIT 1",
        params![collection_id, namespace, page_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(FileEditError::from)
}

fn page_namespace(conn: &Connection, page_id: i64) -> Result<String, FileEditError> {
    conn.query_row(
        "SELECT namespace FROM pages WHERE id = ?1",
        [page_id],
        |row| row.get(0),
    )
    .map_err(FileEditError::from)
}

fn current_timestamp(conn: &Connection) -> Result<String, FileEditError> {
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ', 'now')", [], |row| {
        row.get(0)
    })
    .map_err(FileEditError::from)
}

fn history_on_disk_enabled(conn: &Connection) -> Result<bool, FileEditError> {
    Ok(
        db::read_config_value_or(conn, "corrections.history_on_disk", "false")?
            .eq_ignore_ascii_case("true"),
    )
}

fn history_relative_path(relative_path: &Path, slug: &str, archived_suffix: &str) -> PathBuf {
    let parts = path_parts(relative_path);
    let mut history_path = PathBuf::new();
    match parts.as_slice() {
        ["extracted", ..] => {
            history_path.push("extracted");
        }
        [namespace, "extracted", ..] => {
            history_path.push(namespace);
            history_path.push("extracted");
        }
        _ => {
            history_path.push("extracted");
        }
    }
    history_path.push("_history");
    history_path.push(format!(
        "{}--{archived_suffix}.md",
        slug.replace(['/', '\\'], "--")
    ));
    history_path
}

fn eligible_type(page_type: &str) -> bool {
    ELIGIBLE_TYPES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(page_type))
}

fn normalize_whitespace_lossy(raw_bytes: &[u8]) -> String {
    String::from_utf8_lossy(raw_bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn slug_timestamp(now: &str) -> String {
    now.replace([':', 'T', 'Z'], "-")
        .trim_matches('-')
        .to_string()
}

fn path_parts(path: &Path) -> Vec<&str> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect()
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn derive_slug_from_path(file_path: &Path, root_path: &Path) -> String {
    file_path
        .strip_prefix(root_path)
        .unwrap_or(file_path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

fn infer_type_from_path(file_path: &Path, root_path: &Path) -> Option<String> {
    let parts = path_parts(file_path.strip_prefix(root_path).unwrap_or(file_path));
    let folder = match parts.as_slice() {
        ["extracted", kind, ..] => *kind,
        [_, "extracted", kind, ..] => *kind,
        [kind, ..] => *kind,
        [] => return None,
    };
    let normalized = strip_numeric_prefix(folder).to_lowercase();
    match normalized.as_str() {
        "projects" => Some("project".to_string()),
        "areas" => Some("area".to_string()),
        "resources" => Some("resource".to_string()),
        "archives" => Some("archive".to_string()),
        "journal" | "journals" => Some("journal".to_string()),
        "people" => Some("person".to_string()),
        "companies" | "orgs" => Some("company".to_string()),
        _ => None,
    }
}

fn strip_numeric_prefix(name: &str) -> &str {
    let bytes = name.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index > 0 && index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        &name[index..]
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracted_path_detection_recognizes_namespace_and_history_sidecars() {
        assert!(is_extracted_path(Path::new("extracted/facts/foo.md")));
        assert!(is_extracted_path(Path::new("alpha/extracted/facts/foo.md")));
        assert!(is_history_sidecar_path(Path::new(
            "extracted/_history/facts--foo--2026-05-04T00-00-00Z.md"
        )));
        assert!(is_history_sidecar_path(Path::new(
            "alpha/extracted/_history/facts--foo--2026-05-04T00-00-00Z.md"
        )));
        assert!(!is_extracted_path(Path::new(
            "extracted/_history/facts--foo--2026-05-04T00-00-00Z.md"
        )));
        assert!(!is_extracted_path(Path::new("notes/foo.md")));
    }

    #[test]
    fn history_relative_path_sanitizes_slug_into_single_sidecar_file() {
        assert_eq!(
            history_relative_path(
                Path::new("alpha/extracted/facts/foo.md"),
                "facts/foo",
                "2026-05-04T00-00-00Z"
            ),
            PathBuf::from("alpha")
                .join("extracted")
                .join("_history")
                .join("facts--foo--2026-05-04T00-00-00Z.md")
        );
    }

    #[test]
    fn whitespace_normalization_collapses_format_only_changes() {
        let before = b"---\ntype: fact\n---\nhello\n";
        let after = b"---\n\ntype: fact\n---\nhello  \n";
        assert_eq!(
            normalize_whitespace_lossy(before),
            normalize_whitespace_lossy(after)
        );
    }
}
