//! Superseding-page semantics: managing the `superseded_by` pointer chain
//! between predecessor and successor pages, including head-only enforcement,
//! self-reference rejection, and cross-collection target validation.
//!
//! See also: `collections` for slug resolution across collections, and
//! `conversation::supersede` for the conversation-side write helpers.

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

use crate::core::collections::{self, OpKind, SlugResolution};

/// Failure mode raised when establishing or validating a supersede relationship
/// between two pages.
#[derive(Debug, Error)]
pub enum SupersedeError {
    /// Target slug does not exist in the current collection/namespace.
    #[error("page not found: {slug}")]
    NotFound {
        /// The slug that could not be resolved.
        slug: String,
    },

    /// Target slug matches multiple pages and cannot be uniquely resolved.
    #[error("ambiguous slug: {slug} ({candidates})")]
    Ambiguous {
        /// The ambiguous slug as provided.
        slug: String,
        /// Comma-separated list of candidate canonical slugs.
        candidates: String,
    },

    /// Supersede target is not at the head of its chain — it already has a successor.
    #[error("SupersedeConflictError: page `{slug}` is already superseded by `{successor_slug}`")]
    NonHeadTarget {
        /// Slug of the target that is not at the head of its chain.
        slug: String,
        /// Slug of the existing successor that blocks the new supersede.
        successor_slug: String,
    },

    /// A page tried to mark itself as its own predecessor.
    #[error("SupersedeConflictError: page `{slug}` cannot supersede itself")]
    SelfReference {
        /// Slug of the page that tried to self-reference.
        slug: String,
    },

    /// The writing page is itself already superseded — establishing a new
    /// supersede pointer from a non-head page would create a chain cycle
    /// invisible to head-only retrieval.
    #[error("SupersedeConflictError: page `{slug}` is already superseded by `{successor_slug}` and cannot supersede another page")]
    NonHeadWriter {
        /// Slug of the non-head page that attempted the supersede.
        slug: String,
        /// Slug of the successor that already supersedes the writing page.
        successor_slug: String,
    },

    /// Target lives in a different collection from the new revision.
    #[error("SupersedeConflictError: supersede target `{slug}` must stay in the same collection")]
    CrossCollection {
        /// Slug of the cross-collection target.
        slug: String,
    },

    /// Underlying SQLite failure.
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    /// Collection-resolution failure propagated from [`crate::core::collections`].
    #[error("{0}")]
    Collection(#[from] collections::CollectionError),
}

#[derive(Debug, Clone)]
struct SupersedeTarget {
    id: i64,
    canonical_slug: String,
}

/// Looks up the canonical `<collection>::<slug>` of a successor page id,
/// returning `Ok(None)` when no successor id was supplied.
pub fn successor_slug_by_id(
    conn: &Connection,
    successor_id: Option<i64>,
) -> Result<Option<String>, rusqlite::Error> {
    successor_id
        .map(|page_id| canonical_slug_by_page_id(conn, page_id))
        .transpose()
}

/// Finds the slug of the page that is currently superseded by `successor_id`
/// in the given collection/namespace, or `Ok(None)` if no predecessor exists.
pub fn predecessor_slug_by_successor_id(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    successor_id: i64,
) -> Result<Option<String>, rusqlite::Error> {
    conn.query_row(
        "SELECT slug
         FROM pages
         WHERE collection_id = ?1 AND namespace = ?2 AND superseded_by = ?3
         LIMIT 1",
        params![collection_id, namespace, successor_id],
        |row| row.get(0),
    )
    .optional()
}

/// Adjusts the `superseded_by` pointer for `page_id` so its predecessor
/// matches `supersedes`, clearing any stale predecessor and rejecting invalid
/// targets (self-reference, non-head, cross-collection).
pub fn reconcile_supersede_chain(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    page_id: i64,
    page_slug: &str,
    supersedes: Option<&str>,
) -> Result<(), SupersedeError> {
    let desired_target = desired_target(conn, collection_id, namespace, supersedes)?;
    ensure_target_is_valid(conn, Some(page_id), page_slug, desired_target.as_ref())?;

    let existing_predecessor_id: Option<i64> = conn
        .query_row(
            "SELECT id
             FROM pages
             WHERE collection_id = ?1 AND namespace = ?2 AND superseded_by = ?3
             LIMIT 1",
            params![collection_id, namespace, page_id],
            |row| row.get(0),
        )
        .optional()?;

    let desired_predecessor_id = desired_target.as_ref().map(|target| target.id);
    if existing_predecessor_id == desired_predecessor_id {
        return Ok(());
    }

    if let Some(existing_id) = existing_predecessor_id {
        conn.execute(
            "UPDATE pages
             SET superseded_by = NULL
             WHERE id = ?1 AND superseded_by = ?2",
            params![existing_id, page_id],
        )?;
    }

    if let Some(target) = desired_target {
        let updated = conn.execute(
            "UPDATE pages
             SET superseded_by = ?1
             WHERE id = ?2 AND superseded_by IS NULL",
            params![page_id, target.id],
        )?;
        if updated == 0 {
            let successor_id: Option<i64> = conn
                .query_row(
                    "SELECT superseded_by FROM pages WHERE id = ?1",
                    [target.id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            return Err(SupersedeError::NonHeadTarget {
                slug: target.canonical_slug,
                successor_slug: successor_slug_by_id(conn, successor_id)?
                    .unwrap_or_else(|| page_slug.to_owned()),
            });
        }
    }

    Ok(())
}

/// Dry-run version of [`reconcile_supersede_chain`] that returns the same
/// errors but does not write — used by validators before a page is committed.
pub fn validate_supersede_target(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    current_page_id: Option<i64>,
    page_slug: &str,
    supersedes: Option<&str>,
) -> Result<(), SupersedeError> {
    let desired_target = desired_target(conn, collection_id, namespace, supersedes)?;
    ensure_target_is_valid(conn, current_page_id, page_slug, desired_target.as_ref())
}

fn desired_target(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    supersedes: Option<&str>,
) -> Result<Option<SupersedeTarget>, SupersedeError> {
    supersedes
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| resolve_supersede_target(conn, collection_id, namespace, value))
        .transpose()
}

fn ensure_target_is_valid(
    conn: &Connection,
    current_page_id: Option<i64>,
    page_slug: &str,
    desired_target: Option<&SupersedeTarget>,
) -> Result<(), SupersedeError> {
    let Some(target) = desired_target else {
        return Ok(());
    };

    if current_page_id.is_some_and(|page_id| target.id == page_id) {
        return Err(SupersedeError::SelfReference {
            slug: page_slug.to_owned(),
        });
    }

    let current_successor: Option<i64> = conn
        .query_row(
            "SELECT superseded_by FROM pages WHERE id = ?1",
            [target.id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    if let Some(successor_id) = current_successor {
        if Some(successor_id) != current_page_id {
            return Err(SupersedeError::NonHeadTarget {
                slug: target.canonical_slug.clone(),
                successor_slug: canonical_slug_by_page_id(conn, successor_id)?,
            });
        }
    }

    // Cycle guard: a page that is itself superseded may not establish a NEW
    // supersede pointer — writing chain-tail A with `supersedes: B` would
    // produce a headless A<->B cycle invisible to head-only retrieval
    // (mirrors the non-head writer rejection in conversation/file_edit.rs).
    // Re-asserting an existing predecessor link (the target already points
    // at the writer, i.e. `current_successor == current_page_id`) stays
    // allowed so idempotent re-ingest of exported chains keeps working.
    if let Some(page_id) = current_page_id {
        if current_successor != Some(page_id) {
            let writer_successor: Option<i64> = conn
                .query_row(
                    "SELECT superseded_by FROM pages WHERE id = ?1",
                    [page_id],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if let Some(successor_id) = writer_successor {
                return Err(SupersedeError::NonHeadWriter {
                    slug: page_slug.to_owned(),
                    successor_slug: canonical_slug_by_page_id(conn, successor_id)?,
                });
            }
        }
    }

    Ok(())
}

fn resolve_supersede_target(
    conn: &Connection,
    collection_id: i64,
    namespace: &str,
    raw_slug: &str,
) -> Result<SupersedeTarget, SupersedeError> {
    let normalized_slug = if raw_slug.contains("::") {
        match collections::parse_slug(conn, raw_slug, OpKind::Read)? {
            SlugResolution::Resolved {
                collection_id: resolved_collection_id,
                slug,
                ..
            } => {
                if resolved_collection_id != collection_id {
                    return Err(SupersedeError::CrossCollection {
                        slug: raw_slug.to_owned(),
                    });
                }
                slug
            }
            SlugResolution::NotFound { slug } => return Err(SupersedeError::NotFound { slug }),
            SlugResolution::Ambiguous { slug, candidates } => {
                return Err(SupersedeError::Ambiguous {
                    slug,
                    candidates: candidates
                        .into_iter()
                        .map(|candidate| candidate.full_address)
                        .collect::<Vec<_>>()
                        .join(", "),
                })
            }
        }
    } else {
        raw_slug.to_owned()
    };

    let target_id: i64 = conn
        .query_row(
            "SELECT id
             FROM pages
             WHERE collection_id = ?1 AND namespace = ?2 AND slug = ?3",
            params![collection_id, namespace, normalized_slug],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| SupersedeError::NotFound {
            slug: raw_slug.to_owned(),
        })?;

    Ok(SupersedeTarget {
        id: target_id,
        canonical_slug: canonical_slug_by_page_id(conn, target_id)?,
    })
}

fn canonical_slug_by_page_id(conn: &Connection, page_id: i64) -> Result<String, rusqlite::Error> {
    conn.query_row(
        "SELECT c.name || '::' || p.slug
         FROM pages p
         JOIN collections c ON c.id = p.collection_id
         WHERE p.id = ?1",
        [page_id],
        |row| row.get(0),
    )
}
