# extraction-queue Specification

## Purpose
TBD - created by syncing change conversation-memory-foundations before archive. Update Purpose after archive.

## Requirements
### Requirement: `extraction_queue` table backs the extraction job pipeline
The system SHALL provide a SQLite-backed `extraction_queue` table with columns `id INTEGER PRIMARY KEY`, `session_id TEXT NOT NULL`, `conversation_path TEXT NOT NULL` (vault-relative path to the day-file the job extracts from), `trigger_kind TEXT NOT NULL CHECK (trigger_kind IN ('debounce', 'session_close', 'manual'))`, `enqueued_at TEXT NOT NULL`, `scheduled_for TEXT NOT NULL`, `attempts INTEGER NOT NULL DEFAULT 0`, `last_error TEXT`, and `status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'running', 'done', 'failed'))`. A partial index `idx_extraction_queue_pending` on `(status, scheduled_for) WHERE status = 'pending'` SHALL exist to make the worker's pending-job poll a single index seek.

#### Scenario: Fresh v8 schema creates the table and the partial index
- **WHEN** `quaid init` creates a fresh v8 database
- **THEN** the `extraction_queue` table exists with the column types and constraints above and the partial index `idx_extraction_queue_pending` exists with the documented `WHERE` clause

#### Scenario: Inserting an invalid `trigger_kind` is rejected
- **WHEN** an insert is attempted with `trigger_kind = 'arbitrary'`
- **THEN** the CHECK constraint rejects the insert and no row is added

### Requirement: Enqueue UPSERTs collapse pending jobs per session
The system SHALL provide an enqueue operation that, given `(session_id, conversation_path, trigger_kind, scheduled_for)`, SHALL collapse to at most one `pending` row per `session_id`. Specifically: if a `pending` row already exists for the `session_id`, the existing row's `scheduled_for` SHALL be updated to the later of the existing value and the new value (later for `debounce`, immediate for `session_close` overriding any later debounce), `trigger_kind` SHALL be updated to whichever has the earliest scheduled_for, and `attempts` SHALL be reset to 0. If no `pending` row exists, a new row SHALL be inserted with `status = 'pending'`.

#### Scenario: 20 turns in 10 seconds collapse to one pending job
- **WHEN** 20 `debounce` enqueues for the same `session_id` arrive across 10 seconds, each with `scheduled_for = now + 5s`
- **THEN** the `extraction_queue` table contains exactly one `pending` row for that session, with `scheduled_for` equal to the latest enqueue's value

#### Scenario: `session_close` enqueue overrides a pending debounce
- **WHEN** a `pending` debounce row exists with `scheduled_for = T+5s` and a `session_close` enqueue arrives at `T+1s` with `scheduled_for = T+1s`
- **THEN** the row's `trigger_kind` becomes `session_close` and `scheduled_for` becomes `T+1s`

#### Scenario: New enqueue does not collapse with non-pending rows
- **WHEN** a `running` row exists for `session_id="s1"` and a new `debounce` enqueue for `s1` arrives
- **THEN** a new `pending` row is inserted alongside the `running` row, and both coexist until the running job transitions to `done` or `failed`

### Requirement: Pending jobs are claimed in `scheduled_for` order
The system SHALL provide a worker-facing dequeue operation that selects the `pending` row with the smallest `scheduled_for <= now()`, atomically transitions it to `status = 'running'`, and returns its details. If no row qualifies, the dequeue SHALL return no job. The atomic transition SHALL be safe against concurrent dequeues such that a single job is claimed by exactly one worker.

#### Scenario: Earliest scheduled job is dequeued first
- **WHEN** two pending rows exist with `scheduled_for` values `T+3s` and `T+5s` and the wall clock is `T+10s`
- **THEN** the dequeue returns the `T+3s` row first, transitions it to `running`, and a subsequent dequeue returns the `T+5s` row

#### Scenario: Future-scheduled jobs are not dequeued
- **WHEN** a pending row has `scheduled_for = T+30s` and the wall clock is `T+5s`
- **THEN** dequeue returns no job

#### Scenario: Concurrent dequeues claim distinct rows
- **WHEN** two workers call dequeue concurrently against a single pending row
- **THEN** exactly one worker claims the row and observes `running`, and the other worker observes no available job

### Requirement: Job completion and failure update accounting
The worker SHALL transition a claimed job to `status = 'done'` on success or to `status = 'pending'` (with `attempts += 1` and `last_error` populated) on retriable failure. Completion and failure transitions SHALL be bound to the currently claimed lease attempt, not `job_id` alone, so a stale worker cannot close a re-leased row after lease expiry. After the third failed attempt the job SHALL be transitioned to `status = 'failed'` and SHALL no longer be eligible for dequeue. The retry cap SHALL be configurable via `extraction.max_retries` (default `3`).

#### Scenario: Retriable failure increments attempts and re-pends the job
- **WHEN** a worker completes its run with a retriable failure on a row with `attempts = 0`
- **THEN** the row's `status` becomes `pending`, `attempts` becomes `1`, `last_error` is populated, and the row is eligible for re-dequeue

#### Scenario: Third failure marks the job failed
- **WHEN** a worker completes its run with a retriable failure on a row with `attempts = 2` and `extraction.max_retries = 3`
- **THEN** the row's `status` becomes `failed` and `attempts` becomes `3`, and subsequent dequeues do not return it

#### Scenario: Successful completion marks the job done
- **WHEN** a worker completes its run successfully
- **THEN** the row's `status` becomes `done` and the row is no longer eligible for dequeue

#### Scenario: Stale worker cannot finish a re-leased job
- **WHEN** a worker's lease expires, another worker re-claims the same row, and the stale worker later reports success or failure for the old claim
- **THEN** the stale transition is rejected and the newer lease remains authoritative

### Requirement: Queue persistence survives daemon restart
The system SHALL persist `extraction_queue` rows durably (SQLite WAL, the project's existing durability mode). On `quaid serve` restart, `pending` rows SHALL remain in `pending` and SHALL be dequeued in normal order; `running` rows that were claimed by a worker that did not complete SHALL be re-eligible for dequeue after the shipped fixed 300-second lease-expiry interval so that a crashed worker does not orphan a row indefinitely.

#### Scenario: Pending rows survive a daemon restart
- **WHEN** `quaid serve` is killed while pending rows exist and is then restarted
- **THEN** the previously pending rows are still present with `status = 'pending'` and are dequeued in normal `scheduled_for` order

#### Scenario: Stale running rows recover after lease expiry
- **WHEN** a `running` row's `scheduled_for + 300 seconds` has passed without a `done` or `failed` transition
- **THEN** the row is re-eligible for dequeue and the next claim transitions it back to `running` with `attempts += 1`

