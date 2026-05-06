## ADDED Requirements

### Requirement: A single in-process worker drains the queue inside `quaid serve`
The system SHALL run a single extraction worker as a long-lived task inside `quaid serve` whenever extraction is enabled (and not runtime-disabled per `slm-runtime`). The worker SHALL claim pending jobs via the dequeue contract defined in proposal #1, process them serially, and report success or failure via the existing accounting paths. Concurrent multi-worker operation SHALL NOT be introduced in this proposal — single-worker is the v1 model.

#### Scenario: Worker starts when daemon has extraction enabled
- **WHEN** `quaid serve` starts with `extraction.enabled = true`
- **THEN** a single worker task is running and polling the queue at the configured cadence

#### Scenario: Worker stops when extraction is runtime-disabled
- **WHEN** the daemon transitions to runtime-disabled (e.g. SLM panic or missing model file)
- **THEN** the worker stops claiming new jobs but the daemon process continues serving non-extraction MCP tools

### Requirement: Worker advances the conversation file's frontmatter cursor on success
On successful extraction (SLM call, parse, resolve, fact-page write all succeeded for a window), the worker SHALL update the conversation file's frontmatter `last_extracted_turn` to the highest ordinal in the just-processed new-turns range and `last_extracted_at` to the current timestamp. The cursor write SHALL be persisted before the queue job transitions to `done` so a crash between cursor write and queue write reprocesses the same window without producing duplicate facts (deduplicated by `fact-resolution`).

#### Scenario: Successful window advances the cursor before the job finishes
- **WHEN** the worker successfully extracts a window covering turns 11..15
- **THEN** the conversation file's `last_extracted_turn` becomes `15` on disk before the queue job transitions to `done`

#### Scenario: Crash between cursor write and queue done is recoverable
- **WHEN** the worker writes the cursor for turns 11..15, then crashes before transitioning the queue row to `done`
- **THEN** on daemon restart, the lease-expiry mechanism (proposal #1) re-eligibilises the row; the next dequeue re-runs the same window; `fact-resolution`'s dedup path drops any newly-emitted facts that match the previously-written ones

### Requirement: Idle-timer auto-fires `session_close` enqueue
The system SHALL track per-session "last turn arrival" timestamps. When a session's elapsed idle time exceeds `extraction.idle_close_ms` (default `60000`), the system SHALL enqueue a `session_close` extraction job for that session and SHALL update the corresponding day-file's frontmatter `status` to `closed`. The idle timer SHALL reset on every `memory_add_turn` call for the session.

#### Scenario: Idle session auto-closes after the timeout
- **WHEN** a session's last turn arrived at time T and `extraction.idle_close_ms = 60000`
- **THEN** at approximately T + 60s, the system enqueues a `session_close` job and the day-file's `status` becomes `closed`

#### Scenario: Activity resets the idle timer
- **WHEN** a session receives a turn at T, then another at T + 30s
- **THEN** the auto-close fires at approximately T + 30s + 60s, not T + 60s

### Requirement: Hourly janitor purges old `done` and `failed` queue rows
The system SHALL run an hourly janitor inside `quaid serve` that deletes `extraction_queue` rows with `status IN ('done', 'failed')` and `enqueued_at` older than 30 days. The janitor SHALL be cancellable on daemon shutdown and SHALL NOT block worker activity. The retention window (30 days default) SHALL be configurable via `extraction.retention_days`.

#### Scenario: 31-day-old done row is deleted
- **WHEN** the janitor runs and a `done` row's `enqueued_at` is 31 days in the past
- **THEN** the row is deleted

#### Scenario: 1-day-old done row is preserved
- **WHEN** the janitor runs and a `done` row's `enqueued_at` is 1 day in the past
- **THEN** the row is unchanged

#### Scenario: Pending and running rows are never purged regardless of age
- **WHEN** the janitor runs and the queue contains a `pending` row from 60 days ago (e.g. due to extraction having been disabled)
- **THEN** the row is unchanged
