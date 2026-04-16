# Orchestration: Leela — Search/Embed/Query Revision (ACCEPTED)

**Timestamp:** 2026-04-14T04:56:03Z  
**Coordinator:** Scribe  
**Agent:** Leela (Revision Engineering)  
**Directive:** macro88 (Copilot v0.9.1 Team Mode)

## Mandate

Revise T14–T19 artifact after Professor rejection. Address semantic contract drift, embed CLI
ambiguity, and placeholder truthfulness. Fry locked out of revision cycle; Leela takes over.

## Status

**APPROVED FOR LANDING**

Revision delivered with five key decisions: explicit placeholder contract in inference.rs,
stderr note on every embed invocation, honest status annotations in tasks.md (T14/T18/T19),
blocker sub-bullets for T14, and no code changes to T16–T19 plumbing (stable API).

## Decisions Made

### D1: Explicit Placeholder Contract in Module

`src/core/inference.rs` now carries module-level doc block naming the SHA-256 shim explicitly:
- States: "Hash-based shim, not BGE-small-en-v1.5"
- Lists: Three downstream effects (embed, query, search)
- States: Candle/tokenizers declared but not wired
- States: Public API is stable on Phase 1 ship

Also added `PLACEHOLDER:` caveat to `embed()` and `EmbeddingModel` struct docs.

### D2: Runtime Warning on Every Embed

`embed::run()` emits single `eprintln!` before loop:

```
note: 'bge-small-en-v1.5' is running as a hash-indexed placeholder
(Candle/BGE-small not wired); vector similarity is not semantic until T14 completes
```

Fires on every `gbrain embed` invocation. Stderr preserved; stdout remains parseable.
Scoped block comment explains exact removal step once T14 ships.

### D3: T14 Blocker Sub-Bullets

`[~]` item breakdown:
- `[x]` EmptyInput guard
- `[ ]` Candle tokenize + forward pass — BLOCKER (explicit explanation)

### D4: T18 Honest Status Note

Header now states:
> **T14 dependency (honest status):** Command plumbing ✅ complete. Vectors hash-indexed
> until T14 ships. Runtime stderr note prevents mistaking output for semantic indexing.

Checkboxes remain `[x]` (command does what spec says at API level).

### D5: T19 Honest Status Note

Header now states:
> **T14 dependency (honest status):** Command plumbing ✅ complete. Similarity scores are
> hash-proximity until T14 ships. FTS5 ranking in merged output remains accurate regardless.

Checkboxes remain `[x]` (command does what spec says at API level).

## Validation

- `cargo test`: 115 passed, 0 failed (baseline maintained)
- `cargo check`: clean
- All plumbing stable; tests pass unmodified
- Stderr warnings not captured by harness

## What T14 Completion Requires (Out of Scope)

1. Obtain BGE-small-en-v1.5 weights and tokenizer
2. Decide: `include_bytes!()` (larger binary, offline) vs `online-model` (smaller, downloads on first run)
3. Wire candle-core/-nn/-transformers; replace hash loop with Candle tokenizer + BertModel
4. Remove stderr warning and `PLACEHOLDER:` caveats once model verified
5. Existing tests already correct shape (384-dim, L2-norm ≈ 1.0); will pass with real model

## Outcome

Phase 1 search/embed/query lane now ready for Phase 1 ship gate. Placeholders documented,
contracts truthful, tests stable. Semantic search blocker explicitly deferred to Phase 2.

**Next step:** Commit to main branch.
