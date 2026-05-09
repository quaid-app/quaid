## ADDED Requirements

### Requirement: Crate-level documentation

The crate root (`src/lib.rs`) SHALL carry a multi-paragraph `//!` doc comment that orients a first-time rustdoc reader on what Quaid is, the top-level module structure (`core`, `mcp`, `commands`), and where to start reading for the two primary audiences (library consumers and codebase maintainers).

#### Scenario: rustdoc landing page is non-empty

- **WHEN** a developer runs `cargo doc --no-deps --open` against the repo
- **THEN** the crate's rustdoc landing page displays an introduction paragraph describing Quaid, a module-map paragraph naming `core`, `mcp`, and `commands`, and a "where to start" paragraph that points consumers to `mcp::server` and `core::conversation` and points maintainers to `core::db` and `core::vault_sync`

#### Scenario: crate doc references modules that exist

- **WHEN** `cargo doc --no-deps` is run with `RUSTDOCFLAGS="-D warnings"`
- **THEN** every intra-doc link in the crate `//!` block resolves successfully and the build exits with status 0

### Requirement: Crate-wide missing-docs lint

The crate root SHALL declare `#![warn(missing_docs)]` so that any `pub` item under the crate without a `///` doc produces a rustdoc warning.

#### Scenario: undocumented public item produces a warning

- **GIVEN** a hypothetical `pub fn` is added under `src/core/` without a `///` comment
- **WHEN** `cargo doc --no-deps` is run
- **THEN** the build emits a `missing_docs` warning naming the offending item

#### Scenario: warning is promoted to error in CI

- **WHEN** CI runs the rustdoc job with `RUSTDOCFLAGS="-D warnings"`
- **THEN** any `missing_docs` warning fails the job

### Requirement: Module-level documentation under `core/` and `mcp/`

Every `.rs` file under `src/core/` and `src/mcp/` (including their submodules, e.g., `core/conversation/*.rs` and any modules introduced by the `decompose-vault-sync-module` and `decompose-mcp-server-module` changes) SHALL begin with a `//!` doc block that contains (a) one paragraph describing the module's single responsibility and (b) a "see also" line referencing at least one adjacent module via an intra-doc link.

#### Scenario: module file opens with a //! block

- **WHEN** a maintainer opens any file under `src/core/` or `src/mcp/`
- **THEN** the first non-whitespace lines of the file are `//!` doc comments

#### Scenario: see-also links resolve

- **WHEN** `cargo doc --no-deps` is run with `RUSTDOCFLAGS="-D warnings"`
- **THEN** every intra-doc link inside a `//!` block under `core/` or `mcp/` resolves to an existing item and the build exits with status 0

### Requirement: Public-item documentation under `core/` and `mcp/`

Every `pub fn`, `pub struct`, `pub enum`, `pub trait`, and every `pub` method on a public impl declared under `src/core/` or `src/mcp/` SHALL carry a `///` doc comment containing at least one full sentence that describes the item's intent, role, or contract — not merely restating its signature. Public fields of public structs and public variants of public enums SHALL likewise be documented when the lint requires it.

#### Scenario: public function has intent doc

- **WHEN** a `pub fn` is declared under `src/core/` or `src/mcp/`
- **THEN** the immediately preceding lines contain a `///` block whose content describes what the function does or is for, not just its parameter list

#### Scenario: lint enforces presence

- **WHEN** `cargo doc --no-deps` is run with `RUSTDOCFLAGS="-D warnings"` against the crate
- **THEN** zero `missing_docs` warnings are reported for items reachable through `crate::core` or `crate::mcp`

### Requirement: Documentation scope boundary

Items outside `src/lib.rs`, `src/core/`, and `src/mcp/` — specifically `src/main.rs`, `src/commands/`, build scripts, examples, and tests — SHALL NOT be required to carry doc comments by this contract, and the `missing_docs` lint SHALL NOT fail the build because of items in those locations.

#### Scenario: commands module is exempt

- **WHEN** `cargo doc --no-deps` is run with `RUSTDOCFLAGS="-D warnings"`
- **THEN** no `missing_docs` warning is reported for any item declared under `src/commands/` or `src/main.rs`

#### Scenario: tests and examples are exempt

- **WHEN** `cargo doc --no-deps` is run with `RUSTDOCFLAGS="-D warnings"`
- **THEN** no `missing_docs` warning is reported for items in `tests/` or `examples/`

### Requirement: CI gate on rustdoc warnings

Continuous integration SHALL run `cargo doc --no-deps` with `RUSTDOCFLAGS="-D warnings"` on every pull request, and SHALL exercise both default and `online-model` feature configurations so that feature-gated `pub` items are also covered.

#### Scenario: default-features doc build is gated

- **WHEN** a pull request is opened
- **THEN** CI runs `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` and the PR cannot merge if that step fails

#### Scenario: online-model feature doc build is gated

- **WHEN** a pull request is opened
- **THEN** CI runs `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --no-default-features --features bundled,online-model` and the PR cannot merge if that step fails

#### Scenario: missing doc on new public item blocks merge

- **GIVEN** a developer adds a new `pub fn` under `src/core/` in a pull request and forgets to add a `///` doc
- **WHEN** CI runs the rustdoc job
- **THEN** the job fails with a `missing_docs` error and the PR is blocked from merging
