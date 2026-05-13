//! Library half of Quaid: SQLite-backed storage, hybrid retrieval, embeddings,
//! and the vault-sync, conversation, and quarantine subsystems that sit on top
//! of them. Everything reachable from the CLI commands (`src/commands/`) and
//! the MCP server (`src/mcp/`) lives here.
//!
//! See also: `db` and `types` for the storage layer; `markdown`, `chunking`,
//! and `links` for content parsing; `fts`, `inference`, `search`, and
//! `progressive` for retrieval; `graph`, `assertions`, `novelty`, and `gaps`
//! for derived knowledge views; `palace`, `namespace`, `collections`, and
//! `supersede` for organization and lifecycle; `vault_sync`, `conversation`,
//! `quarantine`, `reconciler`, `raw_imports`, and `migrate` for ingestion,
//! restore, and import/export; `file_state`, `fs_safety`, `ignore_patterns`,
//! and `page_uuid` for filesystem support utilities.

/// Heuristic contradiction detection against existing pages.
pub mod assertions;
/// Page-to-chunk splitting for embedding and progressive retrieval.
pub mod chunking;
/// Collection (per-vault) bookkeeping and isolation.
pub mod collections;
/// Conversation lifecycle: SLM extraction, supersede, format, idle close.
pub mod conversation;
/// SQLite connection setup, schema init, WAL, and `sqlite-vec` loading.
pub mod db;
/// Regex-based entity-pattern extraction routed to assertions only.
pub mod entities;
/// Stat-based file state tracking for detecting external edits.
pub mod file_state;
/// Filesystem-safety helpers: atomic writes, permission checks, path scoping.
pub mod fs_safety;
/// FTS5 full-text search and natural-language query expansion.
pub mod fts;
/// Knowledge-gap log: queries the brain could not answer.
pub mod gaps;
/// N-hop BFS over typed links for neighborhood graph queries.
pub mod graph;
/// `.gitignore`-style pattern matching used by sync ignore lists.
pub mod ignore_patterns;
/// Candle-based embedding model + `sqlite-vec` k-NN search.
pub mod inference;
/// Typed temporal cross-references between pages.
pub mod links;
/// Markdown frontmatter parsing, section splitting, and page rendering.
pub mod markdown;
/// Normalized export and round-trip import of the vault.
pub mod migrate;
/// Namespace isolation: scoped reads, writes, and searches.
pub mod namespace;
/// Jaccard + cosine novelty / dedup checks before writing new content.
pub mod novelty;
/// Stable per-page UUID derivation and lookup.
pub mod page_uuid;
/// Memory-palace classification: wing/room derivation and intent routing.
pub mod palace;
/// Token-budgeted expansion of search hits into context-window-sized payloads.
pub mod progressive;
/// Quarantine lifecycle for problematic content.
pub mod quarantine;
/// Active-source rotation, retention, and byte-exact restore support.
pub mod raw_imports;
/// Reconciliation of competing pages and revisions.
pub mod reconciler;
/// Hybrid search composition that fuses FTS5, vector, and palace filters.
pub mod search;
/// Superseding-page semantics: redirect chains and freshness ordering.
pub mod supersede;
/// Core domain types: `Page`, `Link`, `Tag`, `SearchResult`, error enums.
pub mod types;
/// Vault sync daemon: watcher, ownership, IPC, write-locking, and restore.
pub mod vault_sync;
