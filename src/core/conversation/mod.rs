//! Conversation pipeline root.
//!
//! `conversation` owns the end-to-end path a raw chat turn travels:
//! the turn is captured to a per-day Markdown file, the session is
//! tracked for idleness and closed, an extraction job is queued, a
//! small language model runs to extract facts/decisions/preferences,
//! the result is written into the page graph, and stale state is
//! periodically cleaned up. The submodules each own one stage of that
//! pipeline so the cross-stage contracts (file format, queue rows,
//! page-edit reconciliation) stay localized.
//!
//! - `format` — on-disk conversation Markdown parse + render and the
//!   `<namespace>/conversations/<date>/<session>.md` path scheme
//! - `turn_writer` — appends turns to today's day-file, manages
//!   per-session locks, and explicitly closes sessions
//! - `idle_close` — in-process idle tracker that converts silence into
//!   session-close + extraction enqueue
//! - `queue` — SQLite-backed extraction job queue with debounce /
//!   session-close / manual triggers and lease recovery
//! - `model_lifecycle` — local-cache resolution and download of SLM
//!   weights (extraction model)
//! - `slm` — Phi-3 SLM runner (load / infer / parse JSON envelope)
//!   wrapped with panic isolation and a lazy reusable handle
//! - `extractor` — orchestrates a single extraction job: prompt build,
//!   SLM call, fact-to-page write
//! - `file_edit` — handles user edits to extracted pages by
//!   superseding the prior version while keeping the live slug
//! - `supersede` — temporal-correction primitives that mark a page
//!   superseded by a newer one
//! - `correction` — multi-turn correction sessions where the user
//!   refines an extracted fact
//! - `janitor` — periodic cleanup of done/failed queue rows and
//!   expired correction sessions
//!
//! See also: `crate::core::vault_sync` for the file-watcher path that
//! consumes extracted pages, and `crate::core::raw_imports` for the
//! byte-exact source-of-truth that backs page reconciliation.

/// Multi-turn correction sessions where the user refines an extracted
/// fact before it lands as a page.
pub mod correction;
/// Orchestrates a single extraction job: prompt assembly, SLM
/// inference, validation, and fact-to-page persistence.
pub mod extractor;
/// Handles user edits to already-extracted pages by superseding the
/// prior version on disk while keeping the live slug stable.
pub mod file_edit;
/// On-disk conversation Markdown format: parse, render, and the
/// canonical `<namespace>/conversations/<date>/<session>.md` paths.
pub mod format;
/// In-process idle tracker that converts session silence into a
/// session-close write plus an extraction enqueue.
pub mod idle_close;
/// Periodic cleanup of completed extraction-queue rows and expired
/// correction sessions.
pub mod janitor;
/// Local-cache resolution and download of small-language-model
/// weights for the extraction model.
pub mod model_lifecycle;
/// SQLite-backed extraction job queue with debounce / session-close /
/// manual triggers and lease recovery.
pub mod queue;
/// Small language model runner (Phi-3 via candle) used to extract
/// structured facts from raw conversation turns.
pub mod slm;
/// In-process GGUF (q4_K_M) SLM runner for the Qwen3 extraction model,
/// selected by [`slm::LazySlmRunner`] when the cached model is a `.gguf`.
pub mod slm_gguf;
/// Temporal-correction primitives that link a newer page to an older
/// one and mark the older one superseded.
pub mod supersede;
/// Appends turns to today's conversation day-file under per-session
/// locks and explicitly closes sessions.
pub mod turn_writer;
