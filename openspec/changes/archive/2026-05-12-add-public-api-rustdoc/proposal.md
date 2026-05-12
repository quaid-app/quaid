## Why

`src/lib.rs` is three lines, the crate has no `//!` introduction, and most files in `src/core/` and `src/mcp/` open with code instead of a one-paragraph module doc — so `cargo doc` produces an essentially blank landing page and a maintainer browsing rustdoc gets no help mapping modules to responsibilities. Per `docs/CODE_REVIEW.md` §6.1 this is the first thing rustdoc readers hit, and is currently unusable. Now is the right time to fix it: proposals #4 (`decompose-vault-sync-module`) and #5 (`decompose-mcp-server-module`) split the two largest files into stable, single-responsibility modules — documenting nine ~600-line modules with clear boundaries is tractable; documenting a 12 KLOC monolith would be wasted work.

## What Changes

- Add a multi-paragraph crate-level `//!` doc to `src/lib.rs` covering: what Quaid is (rewritten for human/rustdoc readers, not agents), the module map (`core` lib internals, `mcp` server + tools, `commands` CLI dispatch), and "where to start reading" pointers (consumers → `mcp/server.rs`, `core/conversation`; maintainers → `core/db.rs`, `core/vault_sync/mod.rs`).
- Add a one-paragraph `//!` module doc to every file under `src/core/` and `src/mcp/` (post #4/#5 split), each focused on the module's single responsibility plus a one-line "see also" pointing at adjacent modules.
- Add `#![warn(missing_docs)]` to `src/lib.rs`, and a `///` doc comment (≥ one sentence) on every `pub fn` / `pub struct` / `pub enum` / `pub trait` reachable from `src/core/` and `src/mcp/`. Match the existing good style in `core/fts.rs` and the recently-documented `core/conversation/queue.rs`.
- Establish a CI-enforceable invariant: `cargo doc --no-deps` builds with zero warnings.
- Out of scope: private items, `src/commands/` (binary entry points covered by clap help text), `src/main.rs`, and example/test files.

## Capabilities

### New Capabilities
- `public-api-docs`: Crate-level documentation contract — `lib.rs` carries a crate `//!` intro and `#![warn(missing_docs)]`; every module under `core/` and `mcp/` opens with a `//!` paragraph; every public item under those modules carries a `///` doc; `cargo doc --no-deps` is warning-free.

### Modified Capabilities
<!-- None: this change adds a documentation contract, it does not modify any existing spec's runtime behavior. -->

## Impact

- **Affected code:** `src/lib.rs` (crate doc + lint); every `.rs` file under `src/core/` and `src/mcp/` (module headers + `pub` item docs).
- **CI:** new check — `cargo doc --no-deps` must succeed with zero warnings; `RUSTDOCFLAGS="-D warnings"` is the natural enforcement knob (slot into the lint job from proposal `add-rust-lints-and-ci-gate` if it has landed, otherwise add a standalone step).
- **Dependencies:** none. No new crates, no behavior change, no schema or wire-format change.
- **Sequencing:** depends on `decompose-vault-sync-module` and `decompose-mcp-server-module` having archived. If either is still in flight, this change should wait — re-documenting moved code is wasted effort.
- **Reviewer load:** large mechanical diff (every `pub` item touched), but each hunk is local and verifiable in isolation; review by module rather than by file.
