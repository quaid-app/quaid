-- brain.db schema — GigaBrain v4
-- Embedded in binary via include_str!("schema.sql") in src/core/db.rs
-- Standalone copy for reference and tooling.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ============================================================
-- brain_config: persisted embedding model metadata
-- ============================================================
CREATE TABLE IF NOT EXISTS brain_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;

-- ============================================================
-- pages: the core content table
-- ============================================================
CREATE TABLE IF NOT EXISTS pages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    slug            TEXT    NOT NULL UNIQUE,
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
    version         INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    updated_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    truth_updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    timeline_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

CREATE INDEX IF NOT EXISTS idx_pages_type     ON pages(type);
CREATE INDEX IF NOT EXISTS idx_pages_slug     ON pages(slug);
CREATE INDEX IF NOT EXISTS idx_pages_updated  ON pages(updated_at);
CREATE INDEX IF NOT EXISTS idx_pages_wing     ON pages(wing);
CREATE INDEX IF NOT EXISTS idx_pages_wing_room ON pages(wing, room);

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
    VALUES (new.id, new.title, new.slug, new.compiled_truth, new.timeline);
END;

CREATE TRIGGER IF NOT EXISTS pages_ad AFTER DELETE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
END;

CREATE TRIGGER IF NOT EXISTS pages_au AFTER UPDATE ON pages BEGIN
    INSERT INTO page_fts(page_fts, rowid, title, slug, compiled_truth, timeline)
    VALUES ('delete', old.id, old.title, old.slug, old.compiled_truth, old.timeline);
    INSERT INTO page_fts(rowid, title, slug, compiled_truth, timeline)
    VALUES (new.id, new.title, new.slug, new.compiled_truth, new.timeline);
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
    model           TEXT    NOT NULL DEFAULT 'BAAI/bge-small-en-v1.5'
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
    valid_from      TEXT    DEFAULT NULL,
    valid_until     TEXT    DEFAULT NULL,
    created_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_until >= valid_from)
);

CREATE INDEX IF NOT EXISTS idx_links_from    ON links(from_page_id);
CREATE INDEX IF NOT EXISTS idx_links_to      ON links(to_page_id);
CREATE INDEX IF NOT EXISTS idx_links_current ON links(valid_until);

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
    CHECK (asserted_by IN ('agent', 'manual', 'import', 'enrichment'))
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
-- ingest_log: idempotency audit trail
-- ============================================================
CREATE TABLE IF NOT EXISTS ingest_log (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    ingest_key    TEXT    NOT NULL UNIQUE,
    source_type   TEXT    NOT NULL,
    source_ref    TEXT    NOT NULL,
    pages_updated TEXT    NOT NULL DEFAULT '[]',
    summary       TEXT    NOT NULL DEFAULT '',
    completed_at  TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
);

-- ============================================================
-- config: mutable runtime defaults
-- ============================================================
CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO config (key, value) VALUES
    ('version',               '4'),
    ('embedding_model',       'BAAI/bge-small-en-v1.5'),
    ('embedding_dimensions',  '384'),
    ('chunk_strategy',        'section'),
    ('search_merge_strategy', 'set-union'),
    ('default_token_budget',  '4000');

-- ============================================================
-- contradictions: detected inconsistencies
-- ============================================================
CREATE TABLE IF NOT EXISTS contradictions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    page_id       INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
    other_page_id INTEGER REFERENCES pages(id) ON DELETE SET NULL,
    type          TEXT    NOT NULL,
    description   TEXT    NOT NULL,
    detected_at   TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
    resolved_at   TEXT    DEFAULT NULL
);

CREATE INDEX IF NOT EXISTS idx_contradictions_page       ON contradictions(page_id);
CREATE INDEX IF NOT EXISTS idx_contradictions_unresolved ON contradictions(resolved_at)
    WHERE resolved_at IS NULL;

-- ============================================================
-- knowledge_gaps: queries the brain couldn't answer well
-- Privacy-safe by default: raw query text is NOT retained
-- unless explicitly approved.  Only query_hash is stored on
-- detection; query_text is populated post-approval.
-- ============================================================
CREATE TABLE IF NOT EXISTS knowledge_gaps (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
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
CREATE INDEX IF NOT EXISTS idx_gaps_unresolved ON knowledge_gaps(resolved_at)
    WHERE resolved_at IS NULL;
