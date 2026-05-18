## MODIFIED Requirements

### Requirement: Idle timer auto-fires session close
The system SHALL maintain an idle-timer per active session. When a session has no new `memory_add_turn` activity for `extraction.idle_close_ms` (default `60000`), the system SHALL enqueue a `session_close` extraction job for that session, marking the corresponding day-file's `status` as `closed`. This SHALL fire even without an explicit `memory_close_session` call. After a session is closed, already closed, missing, or stale beyond the configured prune threshold, the system SHALL remove its idle-tracker entry so the process-global idle registry does not grow indefinitely.

#### Scenario: Idle session auto-closes after the timeout
- **WHEN** a session received its last turn at time T and `extraction.idle_close_ms = 60000`
- **THEN** at approximately T + 60s the system enqueues a `session_close` extraction job for the session, the day-file's frontmatter `status` is updated to `closed`, and the worker processes the job in normal queue order

#### Scenario: Activity within the idle window resets the timer
- **WHEN** a session receives a turn at time T and another turn at time T + 30s
- **THEN** the idle-close fires at approximately T + 30s + 60s (not T + 60s)

#### Scenario: Closed session is removed from idle tracking
- **WHEN** the idle scanner closes a due session or observes that the session is already closed
- **THEN** the `(db_path, namespace, session_id)` entry is removed from the process-global idle tracker

#### Scenario: Stale missing session is pruned
- **WHEN** an idle tracker entry refers to a session whose conversation file can no longer be resolved and the entry is older than the stale prune threshold
- **THEN** the idle scanner removes the tracker entry without enqueueing a duplicate close job
