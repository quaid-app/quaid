## ADDED Requirements

### Requirement: Worker drains the extraction queue with a single in-flight job
The system SHALL run a single extraction worker inside `quaid serve` that polls the `extraction_queue` table (delivered by the `extraction-queue` capability), claims pending jobs in `scheduled_for` order, and processes them one at a time. A single in-flight job SHALL be the v1 model — multi-worker concurrency is not required and SHALL NOT be introduced in this proposal. The worker SHALL consult `extraction.enabled` (and the runtime-disabled state from `slm-runtime`) before each claim; when extraction is disabled, the worker SHALL idle without claiming jobs.

#### Scenario: Worker processes pending jobs in scheduled_for order
- **WHEN** three pending jobs exist with `scheduled_for` values T+1s, T+3s, T+5s and the wall clock is past all three
- **THEN** the worker claims and processes them in T+1s, T+3s, T+5s order, with no two jobs in `running` simultaneously

#### Scenario: Worker idles when extraction is disabled
- **WHEN** `extraction.enabled = false` (or runtime-disabled per `slm-runtime`) and pending jobs exist
- **THEN** the worker does not claim jobs and the pending rows remain in `pending`

### Requirement: Window selection slices new turns plus lookback context
On claiming a job, the worker SHALL read the conversation file at `conversation_path` and parse it. Let `C` be the file's `last_extracted_turn` cursor and `last` be the highest turn ordinal in the file. The worker SHALL extract from turns `[C+1, last]` ("new turns"). If `last - C >= extraction.window_turns`, the worker SHALL slice the new turns into non-overlapping windows of size `window_turns`, processing each in order. If `last - C < window_turns`, the worker SHALL run a single window consisting of the new turns plus up to `window_turns - (last - C)` immediately prior turns as **lookback context** — these prior turns SHALL be visible to the SLM but the SLM SHALL be instructed not to re-extract from them. For `trigger_kind = 'session_close'` jobs, the worker SHALL run a final pass even if `last - C == 0` to flush tail turns.

#### Scenario: Sufficient new turns produce non-overlapping windows
- **WHEN** `last_extracted_turn = 0`, `last = 12`, and `window_turns = 5`
- **THEN** the worker runs three SLM calls over windows `[1..5]`, `[6..10]`, `[11..12]` in order

#### Scenario: Insufficient new turns include lookback context
- **WHEN** `last_extracted_turn = 10`, `last = 12`, and `window_turns = 5`
- **THEN** the worker runs one SLM call with new turns `[11, 12]` and lookback context turns `[8, 9, 10]`, with the prompt explicitly marking the lookback range as not-for-extraction

#### Scenario: Session close flushes even with no new turns
- **WHEN** a `session_close` job is dequeued for a session where `last_extracted_turn == last`
- **THEN** the worker runs a single SLM call with the most recent `window_turns` turns purely as context, but writes no new facts unless the SLM produces them; the cursor remains unchanged

### Requirement: Idle timer auto-fires session close
The system SHALL maintain an idle-timer per active session. When a session has no new `memory_add_turn` activity for `extraction.idle_close_ms` (default `60000`), the system SHALL enqueue a `session_close` extraction job for that session, marking the corresponding day-file's `status` as `closed`. This SHALL fire even without an explicit `memory_close_session` call.

#### Scenario: Idle session auto-closes after the timeout
- **WHEN** a session received its last turn at time T and `extraction.idle_close_ms = 60000`
- **THEN** at approximately T + 60s the system enqueues a `session_close` job for the session, the day-file's frontmatter `status` is updated to `closed`, and the worker processes the job in normal queue order

#### Scenario: Activity within the idle window resets the timer
- **WHEN** a session receives a turn at time T and another turn at time T + 30s
- **THEN** the idle-close fires at approximately T + 30s + 60s (not T + 60s)

### Requirement: Worker advances the conversation file's cursor on success
On successful extraction (SLM call, parse, resolve, fact pages written), the worker SHALL update the conversation file's frontmatter cursor: `last_extracted_turn` SHALL be set to the highest turn ordinal that was in the just-processed window's "new turns" range, and `last_extracted_at` SHALL be set to the current timestamp. The cursor SHALL NOT advance on failure. Cursor updates SHALL be persisted to the file before the worker marks the queue job `done`, so a crash between cursor update and queue update reprocesses without duplicating extraction.

#### Scenario: Successful window advances the cursor
- **WHEN** the worker successfully extracts a window covering new turns 11..15
- **THEN** the conversation file's frontmatter `last_extracted_turn` becomes `15`, `last_extracted_at` becomes the current timestamp, and the queue job transitions to `done` only after that file write completes

#### Scenario: Failed window does not advance the cursor
- **WHEN** the worker's SLM call returns unparseable output for a window covering 11..15 and all retries fail
- **THEN** the conversation file's `last_extracted_turn` remains at its prior value, the queue job transitions to `failed`, and a subsequent `quaid extract <session>` re-runs the same window

### Requirement: `--force` re-extraction resets the cursor
The system SHALL provide a `quaid extract <session-id> [--force]` CLI command. With `--force`, the cursor `last_extracted_turn` SHALL be reset to `0` for all of the session's day-files before the worker re-runs extraction; the worker SHALL re-extract the entire session from scratch. Without `--force`, the command SHALL behave as a manual `session_close` enqueue (running extraction over un-extracted tail turns only). The system SHALL also provide `quaid extract --all [--since <date>]` to re-run extraction for all sessions in the active namespace, optionally filtered to those with day-files dated on or after `<date>`.

#### Scenario: `--force` resets the cursor and re-extracts
- **WHEN** `quaid extract s1 --force` runs against a session whose latest day-file has `last_extracted_turn = 47`
- **THEN** the cursor is reset to `0` across all of `s1`'s day-files, the worker processes all turns from ordinal 1 onward, and the resulting fact set replaces (via supersede or dedup) the prior fact set for that session

#### Scenario: Bare `quaid extract` only catches up tail
- **WHEN** `quaid extract s1` runs without `--force` against a session with `last_extracted_turn = 47` and `last = 50`
- **THEN** the worker enqueues and processes only the window covering turns 48..50; turns 1..47 are not re-extracted

### Requirement: Janitor purges done jobs and expired correction sessions
The system SHALL run an hourly janitor inside `quaid serve` that: (a) deletes `extraction_queue` rows with `status IN ('done', 'failed')` whose `enqueued_at` is older than 30 days, and (b) marks `correction_sessions` rows with `status = 'open' AND expires_at < now()` as `expired`. The janitor SHALL be cancellable on daemon shutdown and SHALL NOT block normal worker activity.

#### Scenario: Old done rows are purged
- **WHEN** the janitor runs and an `extraction_queue` row has `status = 'done'` with `enqueued_at` 31 days ago
- **THEN** the row is deleted

#### Scenario: Expired correction sessions are marked
- **WHEN** the janitor runs and a `correction_sessions` row has `status = 'open'` with `expires_at` 1 minute in the past
- **THEN** the row's status becomes `expired`

#### Scenario: Recent rows are not purged
- **WHEN** the janitor runs and an `extraction_queue` row has `status = 'done'` with `enqueued_at` 1 day ago
- **THEN** the row is unchanged
