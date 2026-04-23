// Collections — named groupings with their own root, ignore patterns, and lifecycle.
//
// Every page belongs to a collection. Collections track vault filesystem state,
// ignore patterns, and write-target designation.

#![allow(dead_code)]

use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ── Collection ────────────────────────────────────────────────

/// A named grouping of pages with its own vault root and ignore patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub state: CollectionState,
    pub writable: bool,
    pub is_write_target: bool,
    pub ignore_patterns: Option<String>,
    pub ignore_parse_errors: Option<String>,
    pub needs_full_sync: bool,
    pub last_sync_at: Option<String>,
    pub active_lease_session_id: Option<String>,
    pub restore_command_id: Option<String>,
    pub restore_lease_session_id: Option<String>,
    pub reload_generation: i64,
    pub watcher_released_session_id: Option<String>,
    pub watcher_released_generation: Option<i64>,
    pub watcher_released_at: Option<String>,
    pub pending_command_heartbeat_at: Option<String>,
    pub pending_root_path: Option<String>,
    pub pending_restore_manifest: Option<String>,
    pub restore_command_pid: Option<i64>,
    pub restore_command_host: Option<String>,
    pub integrity_failed_at: Option<String>,
    pub pending_manifest_incomplete_at: Option<String>,
    pub reconcile_halted_at: Option<String>,
    pub reconcile_halt_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollectionState {
    Active,
    Detached,
    Restoring,
}

impl CollectionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Detached => "detached",
            Self::Restoring => "restoring",
        }
    }
}

impl std::str::FromStr for CollectionState {
    type Err = CollectionError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(Self::Active),
            "detached" => Ok(Self::Detached),
            "restoring" => Ok(Self::Restoring),
            _ => Err(CollectionError::InvalidState {
                state: s.to_owned(),
            }),
        }
    }
}

// ── OpKind ────────────────────────────────────────────────────

/// Classification of operations for bare-slug resolution and interlock enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    /// Non-mutating operation (brain_get, brain_search, brain_list, etc.)
    Read,
    /// Write operation creating a new page (brain_put without expected_version)
    WriteCreate,
    /// Write operation updating an existing page (brain_put with expected_version, brain_link, brain_check, etc.)
    WriteUpdate,
    /// Collection-level admin operation (migrate-uuids, ignore add/remove/clear, etc.)
    WriteAdmin,
}

// ── SlugResolution ────────────────────────────────────────────

/// Result of parsing and resolving a slug input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugResolution {
    /// Slug resolved to a single collection
    Resolved {
        collection_id: i64,
        collection_name: String,
        slug: String,
    },
    /// Slug not found in any collection
    NotFound { slug: String },
    /// Slug is ambiguous (exists in multiple collections)
    Ambiguous {
        slug: String,
        candidates: Vec<AmbiguityCandidate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguityCandidate {
    pub collection_name: String,
    pub full_address: String,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum CollectionError {
    #[error("collection not found: {name}")]
    NotFound { name: String },

    #[error("invalid collection state: {state}")]
    InvalidState { state: String },

    #[error("collection name cannot contain '::'")]
    NameContainsSeparator,

    #[error("collection name cannot be empty")]
    NameEmpty,

    #[error("path traversal attempt: {path}")]
    PathTraversal { path: String },

    #[error("absolute path not allowed: {path}")]
    AbsolutePath { path: String },

    #[error("empty path segment in: {path}")]
    EmptySegment { path: String },

    #[error("NUL byte in path: {path}")]
    NulInPath { path: String },

    #[error("ambiguous slug: {slug} (candidates: {candidates})")]
    Ambiguous { slug: String, candidates: String },

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

// ── Validators ────────────────────────────────────────────────

/// Validate a collection name.
///
/// Rules:
/// - Must not be empty
/// - Must not contain `::`
pub fn validate_collection_name(name: &str) -> Result<(), CollectionError> {
    if name.is_empty() {
        return Err(CollectionError::NameEmpty);
    }
    if name.contains("::") {
        return Err(CollectionError::NameContainsSeparator);
    }
    Ok(())
}

/// Validate a relative path (slug).
///
/// Rules:
/// - Must not contain `..` segments
/// - Must not be absolute (start with `/` or Windows drive letter)
/// - Must not contain empty segments (consecutive slashes)
/// - Must not contain NUL bytes
pub fn validate_relative_path(path: &str) -> Result<(), CollectionError> {
    if path.is_empty() {
        return Err(CollectionError::EmptySegment {
            path: path.to_owned(),
        });
    }

    if path.contains('\0') {
        return Err(CollectionError::NulInPath {
            path: path.to_owned(),
        });
    }

    // Check for absolute paths (Unix and Windows)
    if path.starts_with('/') || path.starts_with('\\') {
        return Err(CollectionError::AbsolutePath {
            path: path.to_owned(),
        });
    }
    if path.len() >= 2 && path.chars().nth(1) == Some(':') {
        // Windows drive letter (C:, D:, etc.)
        return Err(CollectionError::AbsolutePath {
            path: path.to_owned(),
        });
    }

    // Check each segment
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(CollectionError::EmptySegment {
                path: path.to_owned(),
            });
        }
        if segment == ".." {
            return Err(CollectionError::PathTraversal {
                path: path.to_owned(),
            });
        }
    }

    Ok(())
}

// ── CRUD helpers ──────────────────────────────────────────────

/// Fetch a collection by name.
pub fn get_by_name(conn: &Connection, name: &str) -> Result<Option<Collection>, CollectionError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, root_path, state, writable, is_write_target, \
               ignore_patterns, ignore_parse_errors, needs_full_sync, last_sync_at, \
               active_lease_session_id, restore_command_id, restore_lease_session_id, \
               reload_generation, watcher_released_session_id, watcher_released_generation, \
               watcher_released_at, pending_command_heartbeat_at, pending_root_path, \
               pending_restore_manifest, restore_command_pid, restore_command_host, \
               integrity_failed_at, pending_manifest_incomplete_at, reconcile_halted_at, \
               reconcile_halt_reason, created_at, updated_at \
          FROM collections WHERE name = ?1",
    )?;

    let result = stmt
        .query_row([name], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                state: row.get::<_, String>(3)?.parse().unwrap(),
                writable: row.get::<_, i64>(4)? != 0,
                is_write_target: row.get::<_, i64>(5)? != 0,
                ignore_patterns: row.get(6)?,
                ignore_parse_errors: row.get(7)?,
                needs_full_sync: row.get::<_, i64>(8)? != 0,
                last_sync_at: row.get(9)?,
                active_lease_session_id: row.get(10)?,
                restore_command_id: row.get(11)?,
                restore_lease_session_id: row.get(12)?,
                reload_generation: row.get(13)?,
                watcher_released_session_id: row.get(14)?,
                watcher_released_generation: row.get(15)?,
                watcher_released_at: row.get(16)?,
                pending_command_heartbeat_at: row.get(17)?,
                pending_root_path: row.get(18)?,
                pending_restore_manifest: row.get(19)?,
                restore_command_pid: row.get(20)?,
                restore_command_host: row.get(21)?,
                integrity_failed_at: row.get(22)?,
                pending_manifest_incomplete_at: row.get(23)?,
                reconcile_halted_at: row.get(24)?,
                reconcile_halt_reason: row.get(25)?,
                created_at: row.get(26)?,
                updated_at: row.get(27)?,
            })
        })
        .optional()?;

    Ok(result)
}

/// Get the write-target collection (the one with is_write_target = 1).
pub fn get_write_target(conn: &Connection) -> Result<Option<Collection>, CollectionError> {
    let mut stmt = conn.prepare(
        "SELECT id, name, root_path, state, writable, is_write_target, \
               ignore_patterns, ignore_parse_errors, needs_full_sync, last_sync_at, \
               active_lease_session_id, restore_command_id, restore_lease_session_id, \
               reload_generation, watcher_released_session_id, watcher_released_generation, \
               watcher_released_at, pending_command_heartbeat_at, pending_root_path, \
               pending_restore_manifest, restore_command_pid, restore_command_host, \
               integrity_failed_at, pending_manifest_incomplete_at, reconcile_halted_at, \
               reconcile_halt_reason, created_at, updated_at \
          FROM collections WHERE is_write_target = 1",
    )?;

    let result = stmt
        .query_row([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                root_path: row.get(2)?,
                state: row.get::<_, String>(3)?.parse().unwrap(),
                writable: row.get::<_, i64>(4)? != 0,
                is_write_target: row.get::<_, i64>(5)? != 0,
                ignore_patterns: row.get(6)?,
                ignore_parse_errors: row.get(7)?,
                needs_full_sync: row.get::<_, i64>(8)? != 0,
                last_sync_at: row.get(9)?,
                active_lease_session_id: row.get(10)?,
                restore_command_id: row.get(11)?,
                restore_lease_session_id: row.get(12)?,
                reload_generation: row.get(13)?,
                watcher_released_session_id: row.get(14)?,
                watcher_released_generation: row.get(15)?,
                watcher_released_at: row.get(16)?,
                pending_command_heartbeat_at: row.get(17)?,
                pending_root_path: row.get(18)?,
                pending_restore_manifest: row.get(19)?,
                restore_command_pid: row.get(20)?,
                restore_command_host: row.get(21)?,
                integrity_failed_at: row.get(22)?,
                pending_manifest_incomplete_at: row.get(23)?,
                reconcile_halted_at: row.get(24)?,
                reconcile_halt_reason: row.get(25)?,
                created_at: row.get(26)?,
                updated_at: row.get(27)?,
            })
        })
        .optional()?;

    Ok(result)
}

// ── Slug parsing ──────────────────────────────────────────────

/// Parse a slug input and resolve it to a collection.
///
/// Input forms:
/// - `<collection>::<slug>` — explicit collection prefix
/// - `<slug>` — bare slug, resolved based on op_kind
///
/// Returns `Resolved`, `NotFound`, or `Ambiguous`.
pub fn parse_slug(
    conn: &Connection,
    input: &str,
    op_kind: OpKind,
) -> Result<SlugResolution, CollectionError> {
    // Split on first `::`
    if let Some(idx) = input.find("::") {
        let collection_name = &input[..idx];
        let slug = &input[idx + 2..];

        validate_relative_path(slug)?;

        // Explicit collection form
        let collection =
            get_by_name(conn, collection_name)?.ok_or_else(|| CollectionError::NotFound {
                name: collection_name.to_owned(),
            })?;

        return Ok(SlugResolution::Resolved {
            collection_id: collection.id,
            collection_name: collection.name,
            slug: slug.to_owned(),
        });
    }

    // Bare slug form — apply ambiguity-aware resolution
    let slug = input;
    validate_relative_path(slug)?;

    // Count collections
    let collection_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM collections", [], |row| row.get(0))?;

    if collection_count == 0 {
        return Ok(SlugResolution::NotFound {
            slug: slug.to_owned(),
        });
    }

    if collection_count == 1 {
        // Single collection — resolve to it
        let collection = conn.query_row("SELECT id, name FROM collections LIMIT 1", [], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        return Ok(SlugResolution::Resolved {
            collection_id: collection.0,
            collection_name: collection.1,
            slug: slug.to_owned(),
        });
    }

    // Multi-collection brain — count owners of this slug
    let owners: Vec<(i64, String)> = conn
        .prepare(
            "SELECT c.id, c.name FROM collections c \
             INNER JOIN pages p ON p.collection_id = c.id \
             WHERE p.slug = ?1",
        )?
        .query_map([slug], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<Result<Vec<_>, _>>()?;

    match (op_kind, owners.len()) {
        // Read ops
        (OpKind::Read, 0) => Ok(SlugResolution::NotFound {
            slug: slug.to_owned(),
        }),
        (OpKind::Read, 1) => Ok(SlugResolution::Resolved {
            collection_id: owners[0].0,
            collection_name: owners[0].1.clone(),
            slug: slug.to_owned(),
        }),
        (OpKind::Read, _) => {
            let candidates = owners
                .into_iter()
                .map(|(_, name)| AmbiguityCandidate {
                    collection_name: name.clone(),
                    full_address: format!("{}::{}", name, slug),
                })
                .collect();
            Ok(SlugResolution::Ambiguous {
                slug: slug.to_owned(),
                candidates,
            })
        }

        // WriteCreate ops
        (OpKind::WriteCreate, 0) => {
            // No owners — resolve to write-target
            if let Some(target) = get_write_target(conn)? {
                Ok(SlugResolution::Resolved {
                    collection_id: target.id,
                    collection_name: target.name,
                    slug: slug.to_owned(),
                })
            } else {
                Ok(SlugResolution::NotFound {
                    slug: slug.to_owned(),
                })
            }
        }
        (OpKind::WriteCreate, 1) => {
            // One owner
            let write_target = get_write_target(conn)?;
            if let Some(target) = write_target {
                if owners[0].0 == target.id {
                    // Owner is write-target — resolve to it
                    Ok(SlugResolution::Resolved {
                        collection_id: owners[0].0,
                        collection_name: owners[0].1.clone(),
                        slug: slug.to_owned(),
                    })
                } else {
                    // Owner is different collection — ambiguous
                    let candidates = vec![AmbiguityCandidate {
                        collection_name: owners[0].1.clone(),
                        full_address: format!("{}::{}", owners[0].1, slug),
                    }];
                    Ok(SlugResolution::Ambiguous {
                        slug: slug.to_owned(),
                        candidates,
                    })
                }
            } else {
                // No write-target configured
                Ok(SlugResolution::NotFound {
                    slug: slug.to_owned(),
                })
            }
        }
        (OpKind::WriteCreate, _) => {
            // Multiple owners — ambiguous
            let candidates = owners
                .into_iter()
                .map(|(_, name)| AmbiguityCandidate {
                    collection_name: name.clone(),
                    full_address: format!("{}::{}", name, slug),
                })
                .collect();
            Ok(SlugResolution::Ambiguous {
                slug: slug.to_owned(),
                candidates,
            })
        }

        // WriteUpdate and WriteAdmin ops
        (OpKind::WriteUpdate | OpKind::WriteAdmin, 0) => Ok(SlugResolution::NotFound {
            slug: slug.to_owned(),
        }),
        (OpKind::WriteUpdate | OpKind::WriteAdmin, 1) => Ok(SlugResolution::Resolved {
            collection_id: owners[0].0,
            collection_name: owners[0].1.clone(),
            slug: slug.to_owned(),
        }),
        (OpKind::WriteUpdate | OpKind::WriteAdmin, _) => {
            let candidates = owners
                .into_iter()
                .map(|(_, name)| AmbiguityCandidate {
                    collection_name: name.clone(),
                    full_address: format!("{}::{}", name, slug),
                })
                .collect();
            Ok(SlugResolution::Ambiguous {
                slug: slug.to_owned(),
                candidates,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;
    use rusqlite::{params, Connection};

    fn open_resolution_db() -> Connection {
        let conn = db::open(":memory:").unwrap();
        conn.execute("DELETE FROM pages", []).unwrap();
        conn.execute("DELETE FROM collections", []).unwrap();
        conn
    }

    fn insert_collection(conn: &Connection, id: i64, name: &str, is_write_target: bool) {
        conn.execute(
            "INSERT INTO collections (id, name, root_path, state, writable, is_write_target) \
             VALUES (?1, ?2, ?3, 'active', 1, ?4)",
            params![
                id,
                name,
                format!(r"C:\vaults\{name}"),
                if is_write_target { 1 } else { 0 }
            ],
        )
        .unwrap();
    }

    fn collection_id(conn: &Connection, name: &str) -> i64 {
        conn.query_row(
            "SELECT id FROM collections WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn insert_page(conn: &Connection, collection_name: &str, slug: &str) {
        conn.execute(
            "INSERT INTO pages \
                 (collection_id, slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'concept', ?2, '', '', '', '{}', '', '', 1)",
            params![collection_id(conn, collection_name), slug],
        )
        .unwrap();
    }

    #[test]
    fn validate_collection_name_rejects_empty() {
        assert!(validate_collection_name("").is_err());
    }

    #[test]
    fn validate_collection_name_rejects_double_colon() {
        assert!(validate_collection_name("work::notes").is_err());
    }

    #[test]
    fn validate_collection_name_accepts_valid() {
        assert!(validate_collection_name("work").is_ok());
        assert!(validate_collection_name("my-notes").is_ok());
    }

    #[test]
    fn validate_relative_path_rejects_traversal() {
        assert!(validate_relative_path("../etc/passwd").is_err());
        assert!(validate_relative_path("notes/../secrets").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_absolute() {
        assert!(validate_relative_path("/etc/passwd").is_err());
        assert!(validate_relative_path("C:\\Windows").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_empty_segments() {
        assert!(validate_relative_path("notes//meeting").is_err());
    }

    #[test]
    fn validate_relative_path_rejects_nul() {
        assert!(validate_relative_path("notes\0meeting").is_err());
    }

    #[test]
    fn validate_relative_path_accepts_valid() {
        assert!(validate_relative_path("people/alice").is_ok());
        assert!(validate_relative_path("notes/2024/meeting").is_ok());
    }

    mod parse_slug {
        use super::*;

        #[test]
        fn resolves_bare_slug_to_only_collection_in_single_collection_brain() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "default", true);

            let resolution = parse_slug(&conn, "notes/meeting", OpKind::Read).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::Resolved {
                    collection_id: 10,
                    collection_name: "default".to_owned(),
                    slug: "notes/meeting".to_owned(),
                }
            );
        }

        #[test]
        fn resolves_explicit_collection_prefix_even_when_slug_has_no_owner() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);

            let resolution =
                parse_slug(&conn, "memory::notes/meeting", OpKind::WriteUpdate).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::Resolved {
                    collection_id: 20,
                    collection_name: "memory".to_owned(),
                    slug: "notes/meeting".to_owned(),
                }
            );
        }

        #[test]
        fn read_resolves_single_owner_in_multi_collection_brain() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);
            insert_page(&conn, "work", "notes/meeting");

            let resolution = parse_slug(&conn, "notes/meeting", OpKind::Read).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::Resolved {
                    collection_id: 10,
                    collection_name: "work".to_owned(),
                    slug: "notes/meeting".to_owned(),
                }
            );
        }

        #[test]
        fn read_returns_not_found_when_multi_collection_brain_has_no_owner() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);

            let resolution = parse_slug(&conn, "notes/missing", OpKind::Read).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::NotFound {
                    slug: "notes/missing".to_owned(),
                }
            );
        }

        #[test]
        fn read_returns_ambiguous_candidates_for_multi_collection_owner_conflict() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);
            insert_page(&conn, "work", "people/alice");
            insert_page(&conn, "memory", "people/alice");

            let SlugResolution::Ambiguous {
                slug,
                mut candidates,
            } = parse_slug(&conn, "people/alice", OpKind::Read).unwrap()
            else {
                panic!("expected ambiguous resolution");
            };

            candidates.sort_by(|left, right| left.collection_name.cmp(&right.collection_name));

            assert_eq!(slug, "people/alice");
            assert_eq!(
                candidates,
                vec![
                    AmbiguityCandidate {
                        collection_name: "memory".to_owned(),
                        full_address: "memory::people/alice".to_owned(),
                    },
                    AmbiguityCandidate {
                        collection_name: "work".to_owned(),
                        full_address: "work::people/alice".to_owned(),
                    },
                ]
            );
        }

        #[test]
        fn write_create_returns_not_found_when_no_write_target_exists() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", false);
            insert_collection(&conn, 20, "memory", false);

            let resolution = parse_slug(&conn, "notes/new-page", OpKind::WriteCreate).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::NotFound {
                    slug: "notes/new-page".to_owned(),
                }
            );
        }

        #[test]
        fn write_create_routes_missing_slug_to_write_target() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", false);
            insert_collection(&conn, 20, "memory", true);

            let resolution = parse_slug(&conn, "notes/new-page", OpKind::WriteCreate).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::Resolved {
                    collection_id: 20,
                    collection_name: "memory".to_owned(),
                    slug: "notes/new-page".to_owned(),
                }
            );
        }

        #[test]
        fn write_create_returns_ambiguous_when_existing_owner_is_not_write_target() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", false);
            insert_collection(&conn, 20, "memory", true);
            insert_page(&conn, "work", "people/alice");

            let SlugResolution::Ambiguous { slug, candidates } =
                parse_slug(&conn, "people/alice", OpKind::WriteCreate).unwrap()
            else {
                panic!("expected ambiguous resolution");
            };

            assert_eq!(slug, "people/alice");
            assert_eq!(
                candidates,
                vec![AmbiguityCandidate {
                    collection_name: "work".to_owned(),
                    full_address: "work::people/alice".to_owned(),
                }]
            );
        }

        #[test]
        fn write_update_returns_not_found_when_bare_slug_has_no_owner() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);

            let resolution = parse_slug(&conn, "people/missing", OpKind::WriteUpdate).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::NotFound {
                    slug: "people/missing".to_owned(),
                }
            );
        }

        #[test]
        fn write_update_returns_ambiguous_candidates_when_multiple_collections_own_slug() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", true);
            insert_collection(&conn, 20, "memory", false);
            insert_page(&conn, "work", "projects/vault-sync");
            insert_page(&conn, "memory", "projects/vault-sync");

            let SlugResolution::Ambiguous {
                slug,
                mut candidates,
            } = parse_slug(&conn, "projects/vault-sync", OpKind::WriteUpdate).unwrap()
            else {
                panic!("expected ambiguous resolution");
            };

            candidates.sort_by(|left, right| left.collection_name.cmp(&right.collection_name));

            assert_eq!(slug, "projects/vault-sync");
            assert_eq!(
                candidates,
                vec![
                    AmbiguityCandidate {
                        collection_name: "memory".to_owned(),
                        full_address: "memory::projects/vault-sync".to_owned(),
                    },
                    AmbiguityCandidate {
                        collection_name: "work".to_owned(),
                        full_address: "work::projects/vault-sync".to_owned(),
                    },
                ]
            );
        }

        #[test]
        fn write_admin_resolves_single_owner_without_using_write_target_rules() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", false);
            insert_collection(&conn, 20, "memory", true);
            insert_page(&conn, "work", "projects/vault-sync");

            let resolution = parse_slug(&conn, "projects/vault-sync", OpKind::WriteAdmin).unwrap();

            assert_eq!(
                resolution,
                SlugResolution::Resolved {
                    collection_id: 10,
                    collection_name: "work".to_owned(),
                    slug: "projects/vault-sync".to_owned(),
                }
            );
        }

        #[test]
        fn write_admin_returns_ambiguous_candidates_when_multiple_collections_own_slug() {
            let conn = open_resolution_db();
            insert_collection(&conn, 10, "work", false);
            insert_collection(&conn, 20, "memory", true);
            insert_page(&conn, "work", "projects/vault-sync");
            insert_page(&conn, "memory", "projects/vault-sync");

            let SlugResolution::Ambiguous {
                slug,
                mut candidates,
            } = parse_slug(&conn, "projects/vault-sync", OpKind::WriteAdmin).unwrap()
            else {
                panic!("expected ambiguous resolution");
            };

            candidates.sort_by(|left, right| left.collection_name.cmp(&right.collection_name));

            assert_eq!(slug, "projects/vault-sync");
            assert_eq!(
                candidates,
                vec![
                    AmbiguityCandidate {
                        collection_name: "memory".to_owned(),
                        full_address: "memory::projects/vault-sync".to_owned(),
                    },
                    AmbiguityCandidate {
                        collection_name: "work".to_owned(),
                        full_address: "work::projects/vault-sync".to_owned(),
                    },
                ]
            );
        }
    }
}
