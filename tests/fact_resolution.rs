use std::path::Path;

use quaid::core::conversation::supersede::{
    resolve_in_scope_with_similarity, EmbeddingEvidenceFailure, FactResolutionError, Resolution,
};
use quaid::core::db;
use quaid::core::types::{PreferenceStrength, RawFact};
use rusqlite::{params, Connection};

fn open_test_db(path: &Path) -> Connection {
    let conn = db::open(path.to_str().unwrap()).unwrap();
    conn.execute(
        "UPDATE collections
         SET root_path = ?1,
             state = 'active'
         WHERE id = 1",
        [path.parent().unwrap().display().to_string()],
    )
    .unwrap();
    conn
}

fn insert_page(
    conn: &Connection,
    slug: &str,
    kind: &str,
    key_name: &str,
    key_value: &str,
    body: &str,
    superseded_by: Option<i64>,
) -> i64 {
    let frontmatter = serde_json::json!({
        "kind": kind,
        key_name: key_value,
    })
    .to_string();
    conn.execute(
        "INSERT INTO pages
             (collection_id, namespace, slug, uuid, type, title, summary, compiled_truth, timeline,
              frontmatter, wing, room, superseded_by, version)
         VALUES
             (1, '', ?1, ?2, ?3, ?1, ?4, ?4, '', ?5, '', '', ?6, 1)",
        params![
            slug,
            format!("uuid-{slug}"),
            kind,
            body,
            frontmatter,
            superseded_by
        ],
    )
    .unwrap();
    conn.last_insert_rowid()
}

fn preference_fact(summary: &str) -> RawFact {
    RawFact::Preference {
        about: "programming-language".to_string(),
        strength: Some(PreferenceStrength::High),
        summary: summary.to_string(),
    }
}

fn untrustworthy_embedding_error(reason: EmbeddingEvidenceFailure) -> FactResolutionError {
    FactResolutionError::UntrustworthyEmbeddingEvidence {
        kind: "preference".to_string(),
        key_field: "about".to_string(),
        key_value: "programming-language".to_string(),
        reason,
    }
}

#[test]
fn resolve_drops_single_head_near_duplicate_fact() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "existing-rust",
        "preference",
        "about",
        "programming-language",
        "User prefers Rust",
        None,
    );

    let resolution = resolve_in_scope_with_similarity(
        &preference_fact("Matt prefers Rust"),
        &conn,
        1,
        "",
        |_, _| Ok(0.95),
    )
    .unwrap();

    assert!(matches!(
        resolution,
        Resolution::Drop {
            matched_slug,
            ..
        } if matched_slug == "existing-rust"
    ));
}

#[test]
fn resolve_supersedes_single_head_mid_similarity_match() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "existing-rust",
        "preference",
        "about",
        "programming-language",
        "Matt prefers Rust",
        None,
    );

    let resolution = resolve_in_scope_with_similarity(
        &preference_fact("Matt switched to Zig"),
        &conn,
        1,
        "",
        |_, _| Ok(0.55),
    )
    .unwrap();

    assert!(matches!(
        resolution,
        Resolution::Supersede { prior_slug, .. } if prior_slug == "existing-rust"
    ));
}

#[test]
fn resolve_allows_single_head_low_similarity_match_to_coexist() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "existing-rust",
        "preference",
        "about",
        "programming-language",
        "Matt prefers Rust for systems work",
        None,
    );

    let resolution = resolve_in_scope_with_similarity(
        &preference_fact("Matt knows JavaScript well"),
        &conn,
        1,
        "",
        |_, _| Ok(0.3),
    )
    .unwrap();

    assert_eq!(resolution, Resolution::Coexist);
}

#[test]
fn resolve_coexists_when_no_head_matches_key() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "editor-helix",
        "preference",
        "about",
        "editor",
        "Matt uses Helix",
        None,
    );

    let resolution = resolve_in_scope_with_similarity(
        &preference_fact("Matt prefers Rust"),
        &conn,
        1,
        "",
        |_, _| panic!("similarity should not run when there are no matching heads"),
    )
    .unwrap();

    assert_eq!(resolution, Resolution::Coexist);
}

#[test]
fn resolve_refuses_same_key_multi_head_candidate_sets() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "tokyo",
        "fact",
        "about",
        "location",
        "tokyo-body",
        None,
    );
    insert_page(
        &conn,
        "singapore",
        "fact",
        "about",
        "location",
        "singapore-body",
        None,
    );
    insert_page(
        &conn,
        "sydney",
        "fact",
        "about",
        "location",
        "sydney-body",
        None,
    );

    let error = resolve_in_scope_with_similarity(
        &RawFact::Fact {
            about: "location".to_string(),
            summary: "Matt lives in Tokyo".to_string(),
        },
        &conn,
        1,
        "",
        |_, _| panic!("ambiguity refusal must happen before cosine comparison"),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FactResolutionError::AmbiguousMatchingHeads {
            kind,
            key_field,
            key_value,
            candidate_slugs
        } if kind == "fact"
            && key_field == "about"
            && key_value == "location"
            && candidate_slugs == vec!["tokyo", "singapore", "sydney"]
    ));
}

#[test]
fn resolve_fails_closed_when_embeddings_are_hash_shim_only_with_matching_heads() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "existing-rust",
        "preference",
        "about",
        "programming-language",
        "Matt prefers Rust",
        None,
    );

    let error = resolve_in_scope_with_similarity(
        &preference_fact("Matt switched to Zig"),
        &conn,
        1,
        "",
        |_, _| {
            Err(untrustworthy_embedding_error(
                EmbeddingEvidenceFailure::HashShimOnly,
            ))
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FactResolutionError::UntrustworthyEmbeddingEvidence {
            kind,
            key_field,
            key_value,
            reason: EmbeddingEvidenceFailure::HashShimOnly
        } if kind == "preference"
            && key_field == "about"
            && key_value == "programming-language"
    ));
}

#[test]
fn resolve_fails_closed_when_embeddings_are_unavailable_with_matching_heads() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    insert_page(
        &conn,
        "existing-rust",
        "preference",
        "about",
        "programming-language",
        "Matt prefers Rust",
        None,
    );

    let error = resolve_in_scope_with_similarity(
        &preference_fact("Matt switched to Zig"),
        &conn,
        1,
        "",
        |_, _| {
            Err(untrustworthy_embedding_error(
                EmbeddingEvidenceFailure::Unavailable {
                    message: "embedding model is not loaded".to_string(),
                },
            ))
        },
    )
    .unwrap_err();

    assert!(matches!(
        error,
        FactResolutionError::UntrustworthyEmbeddingEvidence {
            kind,
            key_field,
            key_value,
            reason: EmbeddingEvidenceFailure::Unavailable { message }
        } if kind == "preference"
            && key_field == "about"
            && key_value == "programming-language"
            && message == "embedding model is not loaded"
    ));
}

#[test]
fn resolve_ignores_non_head_pages_when_historical_rows_would_otherwise_match() {
    let dir = tempfile::TempDir::new().unwrap();
    let conn = open_test_db(&dir.path().join("memory.db"));
    let current_head_id = insert_page(
        &conn,
        "current-rust",
        "preference",
        "about",
        "language",
        "current-body",
        None,
    );
    insert_page(
        &conn,
        "old-rust",
        "preference",
        "about",
        "language",
        "historical-body",
        Some(current_head_id),
    );

    let resolution = resolve_in_scope_with_similarity(
        &RawFact::Preference {
            about: "language".to_string(),
            strength: None,
            summary: "Matt prefers Rust".to_string(),
        },
        &conn,
        1,
        "",
        |_, body| {
            if body == "historical-body" {
                panic!("historical rows must not be considered resolution candidates");
            }
            Ok(0.55)
        },
    )
    .unwrap();

    assert!(matches!(
        resolution,
        Resolution::Supersede { prior_slug, .. } if prior_slug == "current-rust"
    ));
}
