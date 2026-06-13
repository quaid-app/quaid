#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Embedding semantic-quality tests.
//!
//! Covers the BGE-usage fixes: chunk-size capping with overlapping sub-splits,
//! asymmetric query embedding for the BGE en-v1.5 family, embedder-version
//! driven re-embeds, and — the acceptance test — semantic recall of a fact
//! planted mid-page in a heading-less document, which a single truncated
//! whole-page chunk can never surface.
//!
//! The semantic tests run against the real embedded BGE-small model (default
//! `embedded-model` build channel, fully offline) and skip themselves when
//! only the hash-shim fallback is available.

use quaid::commands::embed;
use quaid::core::chunking::chunk_page;
use quaid::core::db;
use quaid::core::inference::{
    embed as embed_passage, embed_query, embedding_evidence_kind, search_vec,
    EmbeddingEvidenceKind, EMBEDDER_VERSION,
};
use quaid::core::types::{Frontmatter, Page};

// ── Fixtures ─────────────────────────────────────────────────────────────────

fn open_test_db() -> rusqlite::Connection {
    let dir = tempfile::TempDir::new().expect("create temp dir");
    let db_path = dir.path().join("memory.db");
    let conn = db::open(db_path.to_str().unwrap()).expect("open DB");
    // Leak TempDir to keep the database file alive for the test's duration.
    std::mem::forget(dir);
    conn
}

fn insert_page(conn: &rusqlite::Connection, slug: &str, title: &str, truth: &str) {
    conn.execute(
        "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, \
                            frontmatter, wing, room, version) \
         VALUES (?1, ?2, 'note', ?3, '', ?4, '', '{}', 'notes', '', 1)",
        rusqlite::params![
            slug,
            quaid::core::page_uuid::generate_uuid_v7(),
            title,
            truth
        ],
    )
    .expect("insert page");
}

fn test_page(compiled_truth: &str, timeline: &str) -> Page {
    Page {
        slug: "journal/2026-06-01".to_owned(),
        uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_owned(),
        page_type: "note".to_owned(),
        superseded_by: None,
        title: "Journal".to_owned(),
        summary: String::new(),
        compiled_truth: compiled_truth.to_owned(),
        timeline: timeline.to_owned(),
        frontmatter: Frontmatter::new(),
        wing: "notes".to_owned(),
        room: String::new(),
        version: 1,
        created_at: "2026-06-01T00:00:00Z".to_owned(),
        updated_at: "2026-06-01T00:00:00Z".to_owned(),
        truth_updated_at: "2026-06-01T00:00:00Z".to_owned(),
        timeline_updated_at: "2026-06-01T00:00:00Z".to_owned(),
    }
}

/// Deterministic heading-less journal prose of at least `target_bytes` bytes,
/// built from short distinct paragraphs so paragraph-boundary splitting has
/// realistic material to work with.
fn journal_filler(target_bytes: usize, salt: usize) -> String {
    const SENTENCES: [&str; 12] = [
        "The morning started with light rain and a slow commute across the bridge.",
        "Lunch was leftover soup, and the afternoon went to clearing the inbox.",
        "The garden beds need mulch before the first frost arrives next month.",
        "A long phone call with the contractor pushed the kitchen estimate again.",
        "Evening reading covered two more chapters of the sailing memoir.",
        "The dog insisted on the longer loop through the park by the river.",
        "Groceries this week: oat milk, lentils, basil, and far too many snacks.",
        "The neighbors are repainting their fence a surprisingly bright shade of blue.",
        "Spent an hour tuning the squeaky derailleur on the commuter bike.",
        "The book club picked an enormous doorstopper novel for December.",
        "Tried a new pour-over ratio and the coffee finally tasted balanced.",
        "The hallway light flickers again; probably the switch this time.",
    ];

    let mut out = String::new();
    let mut index = salt;
    while out.len() < target_bytes {
        let first = SENTENCES[index % SENTENCES.len()];
        let second = SENTENCES[(index + 5) % SENTENCES.len()];
        out.push_str(&format!("Entry {}. {first} {second}\n\n", index + 1));
        index += 1;
    }
    out
}

fn semantic_backend_available() -> bool {
    matches!(
        embedding_evidence_kind(),
        Ok(EmbeddingEvidenceKind::Semantic)
    )
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    // Embeddings are L2-normalized, so the dot product is the cosine.
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Length in bytes of the longest suffix of `a` that is also a prefix of `b`.
fn suffix_prefix_overlap(a: &str, b: &str) -> usize {
    let max = a.len().min(b.len());
    (1..=max)
        .rev()
        .filter(|&n| b.is_char_boundary(n))
        .find(|&n| a.ends_with(&b[..n]))
        .unwrap_or(0)
}

/// Estimated-token cap mirrored from `src/core/chunking.rs`: chunks must fit
/// the encoder's 512-token window with margin (len/4 estimate ≤ 480).
const MAX_CHUNK_ESTIMATED_TOKENS: usize = 480;

// ── Chunk-size capping ────────────────────────────────────────────────────────

#[test]
fn headingless_10kb_page_yields_multiple_overlapping_chunks_under_cap() {
    let content = journal_filler(10_000, 0);
    let page = test_page(&content, "");

    let chunks = chunk_page(&page);
    let truth_chunks: Vec<_> = chunks
        .iter()
        .filter(|chunk| chunk.chunk_type == "truth_section")
        .collect();

    assert!(
        truth_chunks.len() >= 2,
        "10KB heading-less page must be sub-split into multiple chunks, got {}",
        truth_chunks.len()
    );

    for chunk in &truth_chunks {
        assert!(
            chunk.content.len() / 4 <= MAX_CHUNK_ESTIMATED_TOKENS,
            "chunk exceeds the token-estimate cap: {} bytes (heading {:?})",
            chunk.content.len(),
            chunk.heading_path
        );
        assert!(
            content.contains(&chunk.content),
            "every chunk must be a contiguous slice of the original content"
        );
    }

    // Both ends of the page must be covered.
    let first_sentence = content.lines().next().unwrap();
    let last_sentence = content.trim_end().lines().last().unwrap();
    assert!(truth_chunks[0].content.contains(first_sentence));
    assert!(truth_chunks.last().unwrap().content.contains(last_sentence));

    // Consecutive chunks must overlap so facts near window boundaries are not
    // split across vectors without shared context.
    for pair in truth_chunks.windows(2) {
        let overlap = suffix_prefix_overlap(&pair[0].content, &pair[1].content);
        assert!(
            overlap >= 80,
            "consecutive chunks must share ≥80 bytes of context, got {overlap}"
        );
    }

    // Sub-split chunks carry a part index in their heading path.
    let total = truth_chunks.len();
    assert!(
        truth_chunks[0].heading_path.starts_with("[1/"),
        "first part heading should carry a part index, got {:?}",
        truth_chunks[0].heading_path
    );
    assert!(
        truth_chunks
            .last()
            .unwrap()
            .heading_path
            .ends_with(&format!("[{total}/{total}]")),
        "last part heading should carry the part count, got {:?}",
        truth_chunks.last().unwrap().heading_path
    );
}

#[test]
fn oversized_section_under_heading_is_sub_split_with_suffixed_heading() {
    let body = journal_filler(6_000, 3);
    let content = format!("## Background\n{body}");
    let page = test_page(&content, "");

    let chunks = chunk_page(&page);
    let truth_chunks: Vec<_> = chunks
        .iter()
        .filter(|chunk| chunk.chunk_type == "truth_section")
        .collect();

    assert!(truth_chunks.len() >= 2);
    let total = truth_chunks.len();
    for (index, chunk) in truth_chunks.iter().enumerate() {
        assert_eq!(
            chunk.heading_path,
            format!("Background [{}/{total}]", index + 1),
            "sub-split sections must suffix the heading path with a part index"
        );
        assert!(chunk.content.len() / 4 <= MAX_CHUNK_ESTIMATED_TOKENS);
    }
}

#[test]
fn oversized_timeline_entry_is_sub_split_under_cap() {
    let entry = journal_filler(4_000, 6);
    let timeline = format!("2026-06-01 Long retro\n{entry}\n---\n2026-06-02 Short note");
    let page = test_page("", &timeline);

    let chunks = chunk_page(&page);
    let timeline_chunks: Vec<_> = chunks
        .iter()
        .filter(|chunk| chunk.chunk_type == "timeline_entry")
        .collect();

    assert!(
        timeline_chunks.len() >= 3,
        "oversized timeline entry must be sub-split (got {} chunks)",
        timeline_chunks.len()
    );
    for chunk in &timeline_chunks {
        assert!(chunk.content.len() / 4 <= MAX_CHUNK_ESTIMATED_TOKENS);
    }
}

#[test]
fn small_sectioned_page_chunking_is_unchanged() {
    let page = test_page(
        "## State\nAlice is investing.\n## Assessment\nStrong operator.\n## Network\nKnows top founders.",
        "",
    );

    let chunks = chunk_page(&page);
    let truth_chunks: Vec<_> = chunks
        .iter()
        .filter(|chunk| chunk.chunk_type == "truth_section")
        .collect();

    assert_eq!(truth_chunks.len(), 3);
    assert_eq!(truth_chunks[0].heading_path, "State");
    assert_eq!(truth_chunks[1].heading_path, "Assessment");
    assert_eq!(truth_chunks[2].heading_path, "Network");
}

// ── Asymmetric query embedding (real embedded BGE-small, offline) ───────────

#[test]
fn query_embedding_differs_from_passage_embedding_of_same_text() {
    if !semantic_backend_available() {
        eprintln!("skipping: semantic embedding backend unavailable (hash shim)");
        return;
    }

    let text = "Alice moved to Lisbon in March to join a robotics startup.";
    let passage = embed_passage(text).expect("embed passage");
    let query = embed_query(text).expect("embed query");

    assert_eq!(passage.len(), query.len());
    assert_ne!(
        passage, query,
        "BGE en-v1.5 query embeddings must carry the retrieval instruction prefix"
    );
}

#[test]
fn paraphrase_pair_cosine_exceeds_unrelated_pair() {
    if !semantic_backend_available() {
        eprintln!("skipping: semantic embedding backend unavailable (hash shim)");
        return;
    }

    let query =
        embed_query("How much did Marcus pay for the vintage motorcycle?").expect("embed query");
    let paraphrase =
        embed_passage("Marcus bought the old Triumph motorbike for nine thousand dollars.")
            .expect("embed paraphrase");
    let unrelated = embed_passage("The recipe calls for two cups of flour and a pinch of salt.")
        .expect("embed unrelated");

    let paraphrase_cosine = cosine(&query, &paraphrase);
    let unrelated_cosine = cosine(&query, &unrelated);
    assert!(
        paraphrase_cosine > unrelated_cosine,
        "paraphrase pair ({paraphrase_cosine}) must outscore unrelated pair ({unrelated_cosine})"
    );
}

// ── Embedder-version driven re-embeds ────────────────────────────────────────

fn embedding_row_ids(conn: &rusqlite::Connection) -> Vec<i64> {
    let mut stmt = conn
        .prepare("SELECT id FROM page_embeddings ORDER BY id")
        .expect("prepare embedding id query");
    let rows = stmt
        .query_map([], |row| row.get(0))
        .expect("query embedding ids");
    rows.collect::<Result<Vec<i64>, _>>().expect("collect ids")
}

#[test]
fn embedder_version_mismatch_forces_reembed_of_unchanged_content() {
    let conn = open_test_db();
    insert_page(
        &conn,
        "people/alice",
        "Alice",
        "Alice is investing in climate-tech startups.",
    );

    embed::run(&conn, None, true, false).expect("initial embed");
    let initial_ids = embedding_row_ids(&conn);
    assert!(!initial_ids.is_empty());

    // Unchanged content at the current version is skipped.
    embed::run(&conn, None, false, true).expect("no-op stale embed");
    assert_eq!(embedding_row_ids(&conn), initial_ids);

    // Simulate a store last embedded by an older pipeline version.
    conn.execute(
        "UPDATE quaid_config SET value = '1' WHERE key = 'embedder_version'",
        [],
    )
    .expect("downgrade recorded embedder version");

    embed::run(&conn, None, false, true).expect("stale embed after version change");
    let refreshed_ids = embedding_row_ids(&conn);
    assert_ne!(
        refreshed_ids, initial_ids,
        "an embedder-version mismatch must force a re-embed of unchanged content"
    );

    let recorded: String = conn
        .query_row(
            "SELECT value FROM quaid_config WHERE key = 'embedder_version'",
            [],
            |row| row.get(0),
        )
        .expect("read recorded embedder version");
    assert_eq!(
        recorded,
        EMBEDDER_VERSION.to_string(),
        "a clean full pass must record the current embedder version"
    );
}

// ── Acceptance: mid-page fact recall via the semantic arm ────────────────────

/// THE acceptance test for the embedding-quality fix: a fact planted ~5,000
/// characters into a heading-less page must be returned — ranked first, above
/// related-domain distractor pages, and above the pre-fix score level — by
/// `search_vec`. Before chunk capping, the whole page was one chunk silently
/// truncated at 512 tokens, so the fact contributed nothing to the vector
/// index: under the old pipeline this store scores the journal page 0.414
/// (noise floor) and ranks the office-safe distractor (0.443) above it. With
/// CLS pooling + the query instruction + capped overlapping chunks, the
/// fact-bearing window scores ~0.50 while the distractors drop to ≤0.44.
#[test]
fn fact_planted_mid_page_is_returned_by_search_vec() {
    if !semantic_backend_available() {
        eprintln!("skipping: semantic embedding backend unavailable (hash shim)");
        return;
    }

    let conn = open_test_db();

    let fact =
        "The launch codes are kept inside the red filing cabinet in the basement archive room.";
    let truth = format!(
        "{}{fact}\n\n{}",
        journal_filler(5_000, 0),
        journal_filler(5_000, 7)
    );
    insert_page(&conn, "journal/2026-06-01", "Journal 2026-06-01", &truth);
    insert_page(
        &conn,
        "notes/office-safe",
        "Office safe",
        "The office safe in the records room holds the petty cash box and the master \
         keys for the storage closets.",
    );
    insert_page(
        &conn,
        "notes/office-security",
        "Office security",
        "Building access requires a badge at every entrance. Visitors sign in at the \
         front desk, and security cameras cover the lobby and the parking garage.",
    );
    insert_page(
        &conn,
        "notes/sourdough",
        "Sourdough log",
        "The starter doubled in six hours at room temperature. Next bake: increase \
         hydration to 78 percent and proof overnight in the fridge.",
    );

    embed::run(&conn, None, true, false).expect("embed all pages");

    let results = search_vec("Where are the launch codes kept?", 3, None, None, &conn)
        .expect("vector search");

    assert!(!results.is_empty(), "semantic search returned no results");
    let ranking: Vec<_> = results
        .iter()
        .map(|result| (result.slug.clone(), result.score))
        .collect();
    assert_eq!(
        results[0].slug, "journal/2026-06-01",
        "the page holding the mid-page fact must rank first, got {ranking:?}"
    );
    assert!(
        results[0].score > 0.45,
        "the fact-bearing window must score above the pre-fix noise floor \
         (~0.41 when the fact was truncated away), got {ranking:?}"
    );
}
