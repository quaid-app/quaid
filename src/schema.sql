-- memory.db schema — Quaid v10
-- Embedded in binary via include_str!("schema.sql") in src/core/db.rs
-- Standalone copy for reference and tooling.

PRAGMA journal_mode = WAL;
-- NORMAL drops the per-commit fsync of the WAL file (FULL fsyncs on every
-- commit). Under WAL this is crash-safe — at most the last few committed
-- transactions can be lost on power loss, never database corruption — and
-- removes the dominant cost of bulk ingest / reconcile commits.
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- ============================================================
-- quaid_config: persisted embedding model metadata
-- ============================================================
-- Init writes: model_id, model_alias, embedding_dim, schema_version.
-- Runtime keys:
--   embedder_version — embedding pipeline version (pooling / query
--   instruction / chunking semantics; see EMBEDDER_VERSION in
--   src/core/inference.rs) recorded after a clean `quaid embed --all` or
--   `--stale` pass; a missing or mismatched value makes the next full embed
--   pass refresh every page once.
CREATE TABLE IF NOT EXISTS quaid_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

-- ============================================================
-- collections: named groupings with their own root, ignore patterns
-- ============================================================
CREATE TABLE IF NOT EXISTS collections (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    name                TEXT    NOT NULL UNIQUE CHECK(instr(name, '::') = 0),
    root_path           TEXT    NOT NULL,
    state               TEXT    NOT NULL DEFAULT 'active' CHECK(state IN ('active', 'detached', 'restoring')),
    writable            INTEGER NOT NULL DEFAULT 1,
    is_write_target     INTEGER NOT NULL DEFAULT 0,
    ignore_patterns     TEXT    DEFAULT NULL,
    ignore_parse_errors TEXT    DEFAULT NULL,
    needs_full_sync     INTEGER NOT NULL DEFAULT 0,
    last_sync_at        TEXT    DEFAULT NULL,
    active_lease_session_id   TEXT DEFAULT NULL,
    restore_command_id        TEXT DEFAULT NULL,
    restore_lease_session_id  TEXT DEFAULT NULL,
    reload_generation         INTEGER NOT NULL DEFAULT 0,
    watcher_released_session_id TEXT DEFAULT NULL,
    watcher_released_generation INTEGER DEFAULT NULL,
    watcher_released_at         TEXT DEFAULT NULL,
    pending_command_heartbeat_at TEXT DEFAULT NULL,
    pending_root_path           TEXT DEFAULT NULL,
    pending_restore_manifest    TEXT DEFAULT NULL,
    restore_command_pid         INTEGER DEFAULT NULL,
    restore_command_host        TEXT DEFAULT NULL,
    integrity_failed_at         TEXT DEFAULT NULL,
    pending_manifest_incomplete_at TEXT DEFAULT NULL,
    reconcile_halted_at         TEXT DEFAULT NULL,
    reconcile_halt_reason       TEXT DEFAULT NULL,
    created_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at          TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_write_target
    ON collections(is_write_target) WHERE is_write_target = 1;
CREATE INDEX IF NOT EXISTS idx_collections_restore_state
    ON collections(state, needs_full_sync, reload_generation);
CREATE INDEX IF NOT EXISTS idx_collections_reconcile_halt
    ON collections(reconcile_halted_at) WHERE reconcile_halted_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS serve_sessions (
    session_id    TEXT PRIMARY KEY,
    pid           INTEGER NOT NULL,
    host          TEXT    NOT NULL,
    started_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    heartbeat_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    ipc_path      TEXT    DEFAULT NULL,
    session_type  TEXT    NOT NULL DEFAULT 'serve'
) STRICT;

CREATE INDEX IF NOT EXISTS idx_serve_sessions_heartbeat
    ON serve_sessions(heartbeat_at);

CREATE TABLE IF NOT EXISTS collection_owners (
    collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE,
    session_id    TEXT NOT NULL REFERENCES serve_sessions(session_id) ON DELETE CASCADE,
    claimed_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_collection_owners_session
    ON collection_owners(session_id);

-- ============================================================
-- pages: the core content table
-- ============================================================
CREATE TABLE IF NOT EXISTS pages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    -- DEFAULT 1 routes legacy inserts to the default collection (id=1).
    -- A matching ensure_default_collection() call in db.rs guarantees the row exists.
    collection_id   INTEGER NOT NULL DEFAULT 1 REFERENCES collections(id) ON DELETE CASCADE,
    namespace       TEXT    NOT NULL DEFAULT '',
    slug            TEXT    NOT NULL,
    -- NULL until UUID lifecycle (tasks 5a.*) is fully wired; allows NULL so
    -- legacy INSERT helpers that omit uuid continue to work.
    uuid            TEXT    DEFAULT NULL,
    type            TEXT    NOT NULL,
    -- Valid types: person, company, deal, yc, civic, project, concept, original,
    --              source, media, decision, commitment, action_item
    title           TEXT    NOT NULL,
    summary         TEXT    NOT NULL DEFAULT '',
    compiled_truth  TEXT    NOT NULL DEFAULT '',
    timeline        TEXT    NOT NULL DEFAULT '',
    frontmatter     TEXT    NOT NULL DEFAULT '{}',
    wing            TEXT    NOT NULL DEFAULT '',
    room            TEXT    NOT NULL DEFAULT '',
    superseded_by   INTEGER DEFAULT NULL REFERENCES pages(id),
    version         INTEGER NOT NULL DEFAULT 1,
    quarantined_at  TEXT    DEFAULT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    truth_updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(collection_id, namespace, slug)
);

CREATE INDEX IF NOT EXISTS idx_pages_namespace ON pages(namespace);
-- Partial index: SQLite allows multiple NULLs in unique indexes, but being
-- explicit here avoids confusion when uuid is still unset.
CREATE UNIQUE INDEX IF NOT EXISTS idx_pages_uuid ON pages(uuid) WHERE uuid IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_pages_collection ON pages(collection_id);
CREATE INDEX IF NOT EXISTS idx_pages_type     ON pages(type);
CREATE INDEX IF NOT EXISTS idx_pages_slug     ON pages(collection_id, slug);
CREATE INDEX IF NOT EXISTS idx_pages_updated  ON pages(updated_at);
CREATE INDEX IF NOT EXISTS idx_pages_wing     ON pages(wing);
CREATE INDEX IF NOT EXISTS idx_pages_wing_room ON pages(wing, room);
CREATE INDEX IF NOT EXISTS idx_pages_quarantined ON pages(quarantined_at) WHERE quarantined_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_pages_supersede_head
    ON pages(type, superseded_by) WHERE superseded_by IS NULL;
CREATE INDEX IF NOT EXISTS idx_pages_session
    ON pages(json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id'))
    WHERE json_valid(frontmatter)
      AND json_extract(IIF(json_valid(frontmatter), frontmatter, '{}'), '$.session_id') IS NOT NULL;

CREATE TABLE IF NOT EXISTS namespaces (
    id         TEXT PRIMARY KEY,
    ttl_hours  REAL DEFAULT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
) STRICT;

CREATE TABLE IF NOT EXISTS quarantine_exports (
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    quarantined_at  TEXT    NOT NULL,
    exported_at     TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    output_path     TEXT    NOT NULL,
    PRIMARY KEY (page_id, quarantined_at)
);

CREATE INDEX IF NOT EXISTS idx_quarantine_exports_exported
    ON quarantine_exports(exported_at);

-- ============================================================
-- page_fts: full-text search over compiled_truth + timeline
-- ============================================================
CREATE VIRTUAL TABLE IF NOT EXISTS page_fts USING fts5(
    title,
    slug,
    compiled_truth,
    timeline,
    content='pages',
    content_rowid='id',
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS pages_ai AFTER INSERT ON pages BEGIN
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
    WHERE new.quarantined_at IS NULL;
END;

CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
    WHERE old.quarantined_at IS NULL;
END;

-- Only re-tokenize when an FTS-visible column actually changes, or when the
-- page crosses the quarantine NULL/NOT-NULL boundary (the quarantine flip must
-- still fire so FTS-side quarantine filtering stays correct). Metadata-only
-- writes — superseded_by stamps, version bumps, namespace re-stamping — skip
-- the trigger body entirely, so per-event cost no longer scales with the
-- corpus tokenization cost.
CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages
WHEN old.title IS NOT new.title
    OR old.slug IS NOT new.slug
    OR old.compiled_truth IS NOT new.compiled_truth
    OR old.timeline IS NOT new.timeline
    OR (old.quarantined_at IS NULL) <> (new.quarantined_at IS NULL)
BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    SELECT 'delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline
    WHERE old.quarantined_at IS NULL;
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    SELECT new.id, new.title, new.slug, new.compiled_truth, new.timeline
    WHERE new.quarantined_at IS NULL;
END;

-- ============================================================
-- embedding_models: registry — one active model at all times
-- ============================================================
CREATE TABLE IF NOT EXISTS embedding_models (
    name        TEXT    PRIMARY KEY,
    dimensions  INTEGER NOT NULL,
    vec_table   TEXT    NOT NULL UNIQUE,
    active      INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- Exactly one active model enforced via partial unique index.
CREATE UNIQUE INDEX IF NOT EXISTS idx_embedding_models_one_active
    ON embedding_models(active) WHERE active = 1;

-- Seed default model at init:
--   INSERT INTO embedding_models (name, dimensions, vec_table, active)
--   VALUES ('BAAI/bge-small-en-v1.5', 384, 'page_embeddings_vec_384', 1);
-- Vec table created dynamically in db.rs:
--   CREATE VIRTUAL TABLE IF NOT EXISTS page_embeddings_vec_384 USING vec0(embedding float[384]);

-- ============================================================
-- page_embeddings: chunk metadata for vector search
-- ============================================================
CREATE TABLE IF NOT EXISTS page_embeddings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    model           TEXT    NOT NULL DEFAULT 'Qwen/Qwen3-Embedding-0.6B'
                            REFERENCES embedding_models(name),
    vec_rowid       INTEGER NOT NULL,
    chunk_type      TEXT    NOT NULL,   -- 'truth_section' | 'timeline_entry'
    chunk_index     INTEGER NOT NULL,
    chunk_text      TEXT    NOT NULL,
    content_hash    TEXT    NOT NULL,   -- SHA-256 of chunk_text
    token_count     INTEGER NOT NULL,
    heading_path    TEXT    NOT NULL DEFAULT '',
    last_embedded_at TEXT   NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(model, vec_rowid)
);

CREATE INDEX IF NOT EXISTS idx_embeddings_page   ON page_embeddings(page_id);
CREATE INDEX IF NOT EXISTS idx_embeddings_model  ON page_embeddings(model);
CREATE INDEX IF NOT EXISTS idx_embeddings_lookup ON page_embeddings(model, page_id, chunk_index);

-- ============================================================
-- links: typed temporal cross-references between pages
-- ============================================================
CREATE TABLE IF NOT EXISTS links (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    from_page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    to_page_id      INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    relationship    TEXT    NOT NULL DEFAULT 'related',
    context         TEXT    NOT NULL DEFAULT '',
    source_kind     TEXT    NOT NULL DEFAULT 'programmatic' CHECK(source_kind IN ('wiki_link', 'programmatic', 'frontmatter', 'entity_pattern')),
    edge_weight     REAL    NOT NULL DEFAULT 1.0,
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

CREATE INDEX IF NOT EXISTS idx_links_from    ON links(from_page_id);
CREATE INDEX IF NOT EXISTS idx_links_to      ON links(to_page_id);
CREATE INDEX IF NOT EXISTS idx_links_current ON links(valid_until);
CREATE INDEX IF NOT EXISTS idx_links_source  ON links(source_kind);

-- Partial unique index: derived edges (frontmatter, wiki_link, entity_pattern)
-- collapse to a single row per (from, to, relationship, source_kind). Manual
-- `programmatic` links are intentionally excluded so temporal duplicates remain
-- valid history.
CREATE UNIQUE INDEX IF NOT EXISTS idx_links_unique_derived_edge
    ON links(from_page_id, to_page_id, relationship, source_kind)
    WHERE source_kind IN ('wiki_link', 'frontmatter', 'entity_pattern');

-- ============================================================
-- assertions: heuristic contradiction detection
-- ============================================================
CREATE TABLE IF NOT EXISTS assertions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    subject         TEXT    NOT NULL,
    predicate       TEXT    NOT NULL,
    object          TEXT    NOT NULL,
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    supersedes_id   INTEGER DEFAULT NULL REFERENCES assertions(id),
    confidence      REAL    DEFAULT 1.0,
    asserted_by     TEXT    NOT NULL DEFAULT 'agent',
    source_ref      TEXT    NOT NULL DEFAULT '',
    evidence_text   TEXT    NOT NULL DEFAULT '',
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from),
    CHECK (asserted_by IN ('agent', 'manual', 'import', 'enrichment', 'extraction'))
);

CREATE INDEX IF NOT EXISTS idx_assertions_subj ON assertions(subject);
CREATE INDEX IF NOT EXISTS idx_assertions_pred ON assertions(predicate);

-- ============================================================
-- tags
-- ============================================================
CREATE TABLE IF NOT EXISTS tags (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    tag     TEXT    NOT NULL,
    UNIQUE(page_id, tag)
);

CREATE INDEX IF NOT EXISTS idx_tags_tag     ON tags(tag);
CREATE INDEX IF NOT EXISTS idx_tags_page_id ON tags(page_id);

-- ============================================================
-- raw_data: sidecar data (replaces .raw/ JSON files)
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_data (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    source      TEXT    NOT NULL,
    data        TEXT    NOT NULL,
    fetched_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, source)
);

CREATE INDEX IF NOT EXISTS idx_raw_data_page ON raw_data(page_id);

-- ============================================================
-- timeline_entries: structured timeline rows
-- ============================================================
CREATE TABLE IF NOT EXISTS timeline_entries (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id      INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    date         TEXT    NOT NULL,
    source       TEXT    NOT NULL DEFAULT '',
    summary      TEXT    NOT NULL,
    summary_hash TEXT    NOT NULL DEFAULT '',
    detail       TEXT    NOT NULL DEFAULT '',
    created_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, date, summary_hash)
);

CREATE INDEX IF NOT EXISTS idx_timeline_page ON timeline_entries(page_id);
CREATE INDEX IF NOT EXISTS idx_timeline_date ON timeline_entries(date);

-- ============================================================
-- raw_imports: original file bytes for byte-exact round-trip
-- ============================================================
CREATE TABLE IF NOT EXISTS raw_imports (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id    INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    import_id  TEXT    NOT NULL,
    is_active  INTEGER NOT NULL DEFAULT 1,
    content_hash TEXT  NOT NULL DEFAULT '',
    raw_bytes  BLOB    NOT NULL,
    file_path  TEXT    NOT NULL,
    created_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    UNIQUE(page_id, import_id)
);

CREATE INDEX IF NOT EXISTS idx_raw_imports_page   ON raw_imports(page_id);
CREATE INDEX IF NOT EXISTS idx_raw_imports_active ON raw_imports(page_id, is_active)
    WHERE is_active = 1;

CREATE TABLE IF NOT EXISTS import_manifest (
    import_id   TEXT    PRIMARY KEY,
    source_dir  TEXT    NOT NULL,
    page_count  INTEGER NOT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ============================================================
-- file_state: stat-based change detection for vault sync
-- ============================================================
CREATE TABLE IF NOT EXISTS file_state (
    collection_id   INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    relative_path   TEXT    NOT NULL,
    page_id         INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    mtime_ns        INTEGER NOT NULL,
    ctime_ns        INTEGER DEFAULT NULL,
    size_bytes      INTEGER NOT NULL,
    inode           INTEGER DEFAULT NULL,
    sha256          TEXT    NOT NULL,
    -- Cached frontmatter uuid for duplicate-uuid detection: NULL = not yet
    -- cached (file must be read), '' = file has no frontmatter uuid.
    -- Reset to NULL on every content upsert; refreshed lazily by the
    -- reconciler's duplicate-uuid scan so unchanged files are not re-read.
    frontmatter_uuid TEXT   DEFAULT NULL,
    last_seen_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    last_full_hash_at TEXT  NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (collection_id, relative_path)
);

CREATE INDEX IF NOT EXISTS idx_file_state_sha256 ON file_state(sha256);
CREATE INDEX IF NOT EXISTS idx_file_state_audit ON file_state(last_full_hash_at);
CREATE INDEX IF NOT EXISTS idx_file_state_page ON file_state(page_id);

-- ============================================================
-- embedding_jobs: persistent queue for async embedding work
-- ============================================================
CREATE TABLE IF NOT EXISTS embedding_jobs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id     INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL DEFAULT 0,
    priority    INTEGER NOT NULL DEFAULT 0,
    job_state   TEXT    NOT NULL DEFAULT 'pending'
                        CHECK(job_state IN ('pending', 'running', 'failed')),
    attempt_count INTEGER NOT NULL DEFAULT 0,
    last_error  TEXT    DEFAULT NULL,
    created_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    enqueued_at TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    started_at  TEXT    DEFAULT NULL,
    UNIQUE(page_id)
);

CREATE INDEX IF NOT EXISTS idx_embedding_jobs_queue
    ON embedding_jobs(job_state, priority DESC, enqueued_at);

-- ============================================================
-- extraction_queue: queued conversation extraction jobs
-- ============================================================
CREATE TABLE IF NOT EXISTS extraction_queue (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id        TEXT    NOT NULL,
    conversation_path TEXT    NOT NULL,
    trigger_kind      TEXT    NOT NULL CHECK(trigger_kind IN ('debounce', 'session_close', 'manual')),
    enqueued_at       TEXT    NOT NULL,
    scheduled_for     TEXT    NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 0,
    last_error        TEXT    DEFAULT NULL,
    status            TEXT    NOT NULL DEFAULT 'pending'
                            CHECK(status IN ('pending', 'running', 'done', 'failed'))
);

CREATE INDEX IF NOT EXISTS idx_extraction_queue_pending
    ON extraction_queue(status, scheduled_for) WHERE status = 'pending';

-- ============================================================
-- correction_sessions: bounded fact-correction dialogues
-- ============================================================
CREATE TABLE IF NOT EXISTS correction_sessions (
    correction_id TEXT PRIMARY KEY,
    fact_slug     TEXT    NOT NULL,
    exchange_log  TEXT    NOT NULL CHECK(json_valid(exchange_log) AND json_type(exchange_log) = 'array'),
    turns_used    INTEGER NOT NULL DEFAULT 0 CHECK(turns_used >= 0),
    status        TEXT    NOT NULL DEFAULT 'open'
                          CHECK(status IN ('open', 'committed', 'abandoned', 'expired')),
    created_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    expires_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now', '+1 hour'))
) STRICT;

CREATE INDEX IF NOT EXISTS idx_correction_open
    ON correction_sessions(status, expires_at) WHERE status = 'open';

-- ============================================================
-- config: mutable runtime defaults
-- ============================================================
CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO config (key, value) VALUES
    ('version',               '10'),
    ('embedding_model',       'Qwen/Qwen3-Embedding-0.6B'),
    ('embedding_dimensions',  '1024'),
    ('chunk_strategy',        'section'),
    ('search_merge_strategy', 'set-union'),
    ('search.relevance_floor', '0.0'),
    ('default_token_budget',  '4000'),
    ('memory.location',       'vault-subdir'),
    ('corrections.history_on_disk', 'false'),
    ('extraction.max_retries', '3'),
    ('extraction.enabled', 'false'),
    ('extraction.model_alias', 'qwen3-4b-2507'),
    ('extraction.window_turns', '5'),
    ('extraction.debounce_ms', '5000'),
    ('extraction.idle_close_ms', '60000'),
    ('extraction.retention_days', '30'),
    ('fact_resolution.dedup_cosine_min', '0.92'),
    ('fact_resolution.supersede_cosine_min', '0.4'),
    ('daemon.http.enabled', 'false'),
    ('daemon.http.port', '3112'),
    ('daemon.http.bind', '127.0.0.1'),
    ('daemon.http.trusted_loopback', 'false'),
    ('graph_depth',                  '0'),
    ('graph_distance_decay',         '0.5'),
    ('graph_expansion_max',          '50'),
    ('edge_weight_frontmatter',      '1.0'),
    ('edge_weight_entity_pattern',   '0.7'),
    ('edge_weight_wikilink',         '0.5'),
    -- retrieval-quality-rerank knobs (identity defaults; flipped only after
    -- the documented DAB benchmark gate passes)
    ('search.mmr_lambda',                  '1.0'),
    ('search.max_chunks_per_doc_default',  '0'),
    ('search.cross_ref_boost_weight',      '0.0'),
    ('search.cross_ref_boost_cap',         '0.15'),
    ('search.rerank_extractive',           'false'),
    ('search.rerank_extractive_top_n',     '3'),
    ('search.rerank_extractive_budget_ms', '10'),
    -- knowledge-gap loop: persist caller-provided memory_gap context only
    -- when 'true' (auto-logged query-free diagnostics are always stored)
    ('gaps.store_context',                 'false'),
    -- Outbound secret scrubbing for the MCP read surface (issue #159 phase 1).
    -- 'off' (default) => read payloads cross the wire byte-identical to today;
    -- 'patterns' => deterministic regex/blocklist scrub at the serialization
    -- chokepoint. FTS5/embeddings always index originals (outbound-only).
    ('mcp.redact_outbound',          'off'),
    -- Comma/newline-separated literal secrets to always scrub (e.g. an
    -- internal codename). Empty by default.
    ('mcp.redact_blocklist',         ''),
    -- Minimum embed(type_key) cosine for fuzzy head matching when no exact
    -- type-key match exists during fact resolution.
    ('fact_resolution.key_match_cosine_min', '0.85');

-- ============================================================
-- contradictions: detected inconsistencies
-- ============================================================
CREATE TABLE IF NOT EXISTS contradictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id       INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    other_page_id INTEGER REFERENCES pages(id) ON DELETE CASCADE,
    type          TEXT    NOT NULL,
    description   TEXT    NOT NULL,
    detected_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    resolved_at   TEXT    DEFAULT NULL
);

CREATE INDEX IF NOT EXISTS idx_contradictions_page       ON contradictions(page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_other_page ON contradictions(other_page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_unresolved ON contradictions(resolved_at)
    WHERE resolved_at IS NULL;

-- ============================================================
-- knowledge_gaps: queries the memory engine couldn't answer well
-- Privacy-safe by default: raw query text is NOT retained
-- unless explicitly approved.  Only query_hash is stored on
-- detection; query_text is populated post-approval.
-- ============================================================
CREATE TABLE IF NOT EXISTS knowledge_gaps (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id          INTEGER DEFAULT NULL REFERENCES pages(id) ON DELETE CASCADE,
    query_hash       TEXT    NOT NULL,   -- SHA-256 of original query, always stored
    query_text       TEXT    DEFAULT NULL,  -- raw text retained only after approval
    context          TEXT    NOT NULL DEFAULT '',
    confidence_score REAL    DEFAULT NULL,
    sensitivity      TEXT    NOT NULL DEFAULT 'internal',
    approved_by      TEXT    DEFAULT NULL,
    approved_at      TEXT    DEFAULT NULL,
    redacted_query   TEXT    DEFAULT NULL,
    resolved_at      TEXT    DEFAULT NULL,
    resolved_by_slug TEXT    DEFAULT NULL,
    detected_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (sensitivity IN ('internal', 'external', 'redacted')),
    CHECK (query_text IS NULL OR (approved_by IS NOT NULL AND approved_at IS NOT NULL)),
    CHECK (sensitivity = 'internal' OR (approved_by IS NOT NULL AND approved_at IS NOT NULL))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_gaps_query_hash ON knowledge_gaps(query_hash);
CREATE INDEX IF NOT EXISTS idx_gaps_page ON knowledge_gaps(page_id) WHERE page_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_gaps_unresolved ON knowledge_gaps(resolved_at)
    WHERE resolved_at IS NULL;

-- ============================================================
-- conversation_sessions: per-session append cursor cache
-- Caches the highest turn ordinal and most-recent day-file status
-- per (namespace, session_id) so `append_turn` no longer re-parses
-- every day-file on every turn (O(session^2) over a session's life).
-- The on-disk day-files remain the source of truth; this is a derived
-- cache that is rebuilt from disk if a row is missing.
-- ============================================================
CREATE TABLE IF NOT EXISTS conversation_sessions (
    namespace     TEXT    NOT NULL DEFAULT '',
    session_id    TEXT    NOT NULL,
    max_ordinal   INTEGER NOT NULL DEFAULT 0,
    latest_status TEXT    DEFAULT NULL,
    latest_date   TEXT    DEFAULT NULL,
    updated_at    TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    PRIMARY KEY (namespace, session_id)
);
