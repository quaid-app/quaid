# extraction-control-cli Specification

## Purpose
TBD - created by archiving change slm-extraction-and-correction. Update Purpose after archive.
## Requirements
### Requirement: `quaid extraction` subcommand controls runtime extraction state
The system SHALL provide a `quaid extraction` CLI subcommand group with the following children:

- `quaid extraction enable` — flips `extraction.enabled` to `true` AND eagerly downloads the configured model (delegating to `slm-runtime`'s contract).
- `quaid extraction disable` — flips `extraction.enabled` to `false` without removing model files.
- `quaid extraction status` — prints a human-readable summary of model state, queue depth, active sessions, last-extraction-at per session, and recent failed jobs.

#### Scenario: `enable` followed by `status` shows the loaded state
- **WHEN** `quaid extraction enable` runs successfully and then `quaid extraction status` runs
- **THEN** the status output reports `Extraction enabled: yes`, the model alias and a non-zero resident-memory figure (or "not loaded yet" if no extraction has occurred since enable)

#### Scenario: `status` reports queue depth and active sessions
- **WHEN** the queue contains 2 pending jobs, 0 running, and 1 failed job from the last 24h, and 3 sessions have received turns within the idle window
- **THEN** `quaid extraction status` lists those counts and lists each active session with idle duration

### Requirement: `quaid model pull <alias>` downloads without flipping the flag
The system SHALL provide a `quaid model pull <alias>` CLI subcommand that downloads a model into the local cache without changing `extraction.enabled`. The accepted aliases SHALL include at least `phi-3.5-mini`, `gemma-3-1b`, `gemma-3-4b`, plus any value treated as a full Hugging Face model id by the underlying loader. Successful downloads SHALL verify integrity (hash check) before reporting success.

#### Scenario: Pull caches the model without flipping the flag
- **WHEN** `quaid model pull gemma-3-1b` runs successfully with `extraction.enabled = false`
- **THEN** the model is cached on disk and `extraction.enabled` remains `false`

#### Scenario: Failed integrity check leaves no partial cache
- **WHEN** the download completes but the hash check fails
- **THEN** the partial file is removed from the cache, the command exits non-zero, and a subsequent `quaid model pull <alias>` retry can proceed cleanly

### Requirement: `quaid extract` re-runs extraction from CLI
The system SHALL provide:

- `quaid extract <session-id>` — enqueues an immediate `manual` extraction job for the session; equivalent to the worker's session_close path without modifying the day-file's `status`. Catches up un-extracted tail turns only.
- `quaid extract <session-id> --force` — resets the cursor `last_extracted_turn` to `0` across all of the session's day-files, then enqueues an immediate `manual` job; the worker re-extracts the entire session from scratch.
- `quaid extract --all [--since <date>]` — iterates all sessions in the active namespace; for each, enqueues an immediate `manual` job. With `--since`, restricts to sessions with at least one day-file dated on or after `<date>`.

#### Scenario: Bare `extract` catches up tail turns
- **WHEN** session `s1` has `last_extracted_turn = 47` and `last = 50`, and `quaid extract s1` runs
- **THEN** the worker processes a window covering turns 48..50 and the cursor advances to `50`

#### Scenario: `--force` re-extracts from cursor 0
- **WHEN** `quaid extract s1 --force` runs
- **THEN** the cursor on each of `s1`'s day-files is reset to `0` before the job is enqueued, the worker re-runs extraction over all turns, and the resulting fact set replaces (via supersede or dedup) the prior set

#### Scenario: `--all --since` filters by date
- **WHEN** `quaid extract --all --since 2026-05-01` runs and the namespace has sessions with day-files in April and May
- **THEN** only May-dated sessions have manual extraction jobs enqueued; April-only sessions are skipped

### Requirement: `quaid extraction status` reports failed jobs with actionable detail
The system SHALL include in `quaid extraction status` output a list of jobs with `status = 'failed'` from the last 24 hours, naming the session, the number of attempts, and a truncated `last_error` (≤ 200 characters). The output SHALL be human-readable and SHALL guide the user to either `quaid extract <session> --force` (re-run) or escalation steps (e.g. swapping models via `extraction.model_alias`).

#### Scenario: Failed job is named with attempts and truncated error
- **WHEN** a job for session `s2` failed three attempts ago with `last_error: "JSON parse failure at offset 247: unexpected token..."`
- **THEN** the status output contains a line naming `s2`, `attempts: 3`, and a truncation of the error not exceeding 200 characters

