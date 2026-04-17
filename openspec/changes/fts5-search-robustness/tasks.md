# FTS5 Search Robustness — Implementation Checklist

**Scope:** Apply `sanitize_fts_query` to the `gbrain search` command and MCP `brain_search`
tool; add `--raw` flag for expert FTS5 access; emit `{"error":...}` JSON on error when
`--json` is active.
Closes: #52, #53

---

## Phase A — src/commands/search.rs

- [x] A.1 Import `sanitize_fts_query` from `crate::core::fts` in `src/commands/search.rs`.

- [x] A.2 Add a `raw: bool` parameter to the `run` function signature (alongside existing
  `query`, `wing`, `limit`, `json`).

- [x] A.3 In `run()`, before calling `search_fts`, apply the sanitizer unless `raw` is set:
  ```rust
  let effective_query = if raw { query.to_owned() } else { sanitize_fts_query(query) };
  let results = search_fts(&effective_query, wing.as_deref(), db, limit as usize);
  ```
  Capture the `Result` (do not use `?` here).

- [x] A.4 Handle the result with JSON-aware error output:
  ```rust
  let results = match results {
      Ok(r) => r,
      Err(e) => {
          if json {
              println!("{}", serde_json::json!({"error": e.to_string()}));
          } else {
              return Err(e.into());
          }
          return Ok(());
      }
  };
  ```

- [x] A.5 Wire up the `raw: bool` field in the clap argument struct for `gbrain search`
  (in `src/main.rs` or wherever the `Search` subcommand args are defined):
  - Add `#[arg(long, default_value_t = false)]` `raw: bool` field.
  - Pass it through to `search::run(db, query, wing, limit, json, raw)`.
  - Update the `--help` text to note: "Default mode sanitizes natural-language input.
    Use --raw for expert FTS5 syntax (quoted phrases, boolean operators, wildcards)."

---

## Phase B — src/mcp/server.rs

- [x] B.1 In the `brain_search` tool handler, import `sanitize_fts_query` and apply it
  to the incoming `query` parameter before calling `search_fts`:
  ```rust
  let safe_query = sanitize_fts_query(&query);
  let results = search_fts(&safe_query, wing.as_deref(), conn, limit)?;
  ```

---

## Phase C — src/core/fts.rs

- [x] C.1 Update the `search_fts` doc comment to clarify that `src/commands/search.rs`
  now sanitizes by default, and that the `--raw` flag bypasses sanitization. No logic
  changes to `search_fts` itself.

---

## Phase D — tests

- [x] D.1 Unit test in `src/commands/search.rs` (or `tests/`): call `run()` with
  `raw = false` and query `"what is CLARITY?"` — verify no FTS5 error is returned.

- [x] D.2 Unit test: call `run()` with `raw = false` and query `"it's a stablecoin"` —
  verify no error.

- [x] D.3 Unit test: call `run()` with `raw = false` and query `"gpt-5.4 codex model"` —
  verify no error.

- [x] D.4 Integration test: `gbrain search --json "50% fee reduction"` — verify stdout is
  valid JSON (array or empty array, not an error message).

- [x] D.5 Integration test: `gbrain search --raw --json "?invalid"` — verify stdout is
  valid JSON `{"error": "..."}` and process exits cleanly (not a panic).

- [x] D.6 Add a `brain_search` MCP integration test: send a natural-language query with
  `?` character — verify the tool returns a valid JSON-RPC response (not an error response).

---

## Phase E — verification

- [x] E.1 All tests in Phase D pass. Full `cargo test` suite green.

- [x] E.2 Manually verify `gbrain search "what is CLARITY?"` returns results or empty list
  without crashing. Close issues #52 and #53.

- [x] E.3 Run the following benchmark validation commands (from Kif's v0.9.4 triage) and
  confirm all exit 0 with valid output:
  ```bash
  cargo run --quiet -- init benchmark_issue_check.db
  cargo run --quiet -- --db benchmark_issue_check.db import tests/fixtures
  cargo run --quiet -- --db benchmark_issue_check.db search "what is CLARITY?"
  cargo run --quiet -- --db benchmark_issue_check.db --json search "gpt-5.4 codex model"
  ```
  Clean up `benchmark_issue_check.db` after the run.

- [ ] E.4 DAB rerun checkpoint: run Doug's DAB FTS slice against the v0.9.4 branch. Confirm
  zero crash or parse-error failures in the FTS section. Record the rerun result in the
  issue tracker or release notes before closing the lane.
