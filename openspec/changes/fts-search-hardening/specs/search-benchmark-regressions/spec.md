## Search Benchmark Regressions Spec

### Requirement: command-surface regressions cover Doug's reported failures

The v0.9.4 search hardening lane SHALL add automated regressions at the explicit command surface,
not only around low-level sanitizer helpers.

#### Scenario: benchmark-reported CLI cases are covered

- **WHEN** the automated test suite runs
- **THEN** it includes regressions for `what is CLARITY?`, `it's a stablecoin`,
  `50% fee reduction`, and `gpt-5.4 codex model`
- **AND** those regressions execute the explicit `search` command path

### Requirement: MCP regressions cover the same input class

The automated test suite SHALL cover `brain_search` with the same punctuation and dotted-token
inputs reported in issues #52 and #53.

#### Scenario: MCP search regressions execute

- **WHEN** the MCP test suite runs
- **THEN** it verifies `brain_search` succeeds and returns valid JSON for the benchmark input
  class

### Requirement: release validation includes a benchmark rerun

Closing this lane SHALL require rerun evidence from Doug's benchmark FTS slice, not just unit
tests.

#### Scenario: benchmark validation commands are executed

- **WHEN** the implementation is ready for sign-off
- **THEN** the following commands (or equivalent automated coverage) are run against a benchmark
  database:
  - `cargo run --quiet -- init benchmark_issue_check.db`
  - `cargo run --quiet -- --db benchmark_issue_check.db import tests/fixtures`
  - `cargo run --quiet -- --db benchmark_issue_check.db search "what is CLARITY?"`
  - `cargo run --quiet -- --db benchmark_issue_check.db --json search "gpt-5.4 codex model"`
- **AND** the rerun records zero FTS crash/parse-error failures
