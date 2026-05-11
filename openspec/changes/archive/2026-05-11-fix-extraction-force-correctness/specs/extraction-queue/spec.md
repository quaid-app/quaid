## ADDED Requirements

### Requirement: `extract --force` re-enqueues every day-file in the targeted session

When the operator invokes `quaid extract <session> --force`, the system SHALL enqueue one extraction job per day-file belonging to that session, in chronological (ascending `date`) order, after resetting each day-file's `last_extracted_turn` cursor to `0` and clearing `last_extracted_at`. A forced reset SHALL NOT enqueue only the most recent day-file. For multi-day sessions, the worker SHALL process each enqueued job through the normal `pending → running → done` lifecycle until every day-file has been re-extracted from cursor `0`.

#### Scenario: Multi-day forced reset enqueues every day-file

- **WHEN** `quaid extract s1 --force` runs against a session with day-files dated 2026-05-03, 2026-05-04, and 2026-05-05
- **THEN** three pending jobs are inserted into `extraction_queue` with `trigger_kind = 'manual'`, one per day-file, with `conversation_path` values referencing each day-file in chronological order, and each day-file's `last_extracted_turn` is reset to `0` on disk

#### Scenario: Single-day forced reset behaves identically

- **WHEN** `quaid extract s1 --force` runs against a session with a single day-file dated 2026-05-05
- **THEN** exactly one pending job is enqueued for that day-file and its cursor is reset to `0`

#### Scenario: Worker processes every enqueued day-file to completion

- **WHEN** the worker drains the queue after a multi-day forced reset
- **THEN** every day-file's resulting `last_extracted_turn` advances past every turn in that day-file, and no day-file is left at cursor `0` after queue drain

### Requirement: Queue write transactions remain atomic across commit failure

The system SHALL wrap multi-statement queue mutations (enqueue, dequeue, mark-done, mark-failed, lease-recovery) in a transaction such that, on failure of either the closure or the `COMMIT`, the connection is left in a non-transactional state with no partial mutations applied. Implementations SHALL prefer `rusqlite`'s RAII `Transaction` API with `TransactionBehavior::Immediate` so that commit failure cannot leave the shared `Connection` stuck inside an open transaction.

#### Scenario: Commit failure rolls back without wedging the connection

- **WHEN** a queue write closure succeeds but the underlying `COMMIT` fails (e.g. simulated I/O error)
- **THEN** the in-flight changes are not visible to subsequent reads, the connection is no longer inside a transaction, and a follow-up queue write on the same connection succeeds normally

#### Scenario: Closure failure rolls back without wedging the connection

- **WHEN** a queue write closure returns an error
- **THEN** the in-flight changes are rolled back, the connection is no longer inside a transaction, and a follow-up queue write on the same connection succeeds normally
