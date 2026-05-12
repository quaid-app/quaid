## ADDED Requirements

### Requirement: Worker ownership is single-process-per-database via `daemon` or `serve_host` session type

The system SHALL guarantee that exactly one process per database hosts the extraction-worker thread at any given time. Process selection SHALL be deterministic and registry-driven: when a live `daemon` session is registered in `serve_sessions` (per the `vault-sync` capability), that process SHALL be the unique worker host; otherwise the unique `serve_host` session (promoted atomically from a `serve` registration via the `vault-sync` capability's "atomic runtime-host promotion" requirement) SHALL be the worker host. Processes registered as `'serve'` or `'cli'` SHALL NEVER spawn the extraction worker. The "single in-flight job" constraint already required by the existing `Worker drains the extraction queue with a single in-flight job` requirement remains in force and is unchanged by this addition; this requirement strengthens that constraint from "single-in-flight" to "single-process-per-database" so concurrent processes (multiple `quaid serve` invocations, a daemon plus a serve, etc.) never race for the same queue claim.

#### Scenario: Daemon hosts the worker; concurrent serve does not
- **WHEN** `quaid daemon run` is running and registered as the `daemon` session, and `quaid serve` is invoked against the same database
- **THEN** the daemon process spawns the extraction worker
- **AND** the serve process is registered as `'serve'` and does NOT spawn an extraction worker
- **AND** queue rows enqueued via the serve process are drained by the daemon's worker

#### Scenario: First bare-serve becomes serve_host and hosts the worker; second stays transport-only
- **WHEN** no `daemon` session is registered and two `quaid serve` processes start concurrently
- **THEN** exactly one is promoted to `session_type = 'serve_host'` and spawns the extraction worker
- **AND** the other remains `session_type = 'serve'` and does NOT spawn an extraction worker
- **AND** queue rows are drained by the unique `serve_host`

#### Scenario: Daemon crash hand-off does not double-spawn
- **WHEN** the daemon dies via SIGKILL (no clean unregister) and the daemon session row remains until the 15s liveness threshold passes
- **THEN** during the window before sweep, a newly-invoked `quaid serve` observes the live daemon row, fails the `serve_host` promotion, and registers as `'serve'` (transport-only)
- **AND** after sweep, the next `quaid serve` (or `quaid daemon run`) observes no live runtime-host and successfully claims ownership
- **AND** the queue rows are drained by the new owner without duplicate extraction (existing dedup contract applies if the same window is reprocessed)

### Requirement: Worker honors SIGTERM cooperatively, finishing the current job

When the worker is hosted by `quaid daemon run`, it SHALL observe a shutdown stop flag set by the daemon's SIGTERM handler. On observing the flag, the worker SHALL finish the **current extraction job** — including every window the existing per-job commit model groups into a single queue-row transition — before exiting the loop. The worker SHALL NOT claim a new job after the stop flag is set. This requirement is scoped at *job* granularity rather than *window* granularity to align with the existing extractor's commit model in `src/core/conversation/extractor.rs`, where the conversation file's frontmatter cursor and the `extraction_queue` row's `done` transition are persisted atomically at the job boundary, not the window boundary. Worker-shutdown time SHALL be bounded by one job's worst-case inference latency plus bookkeeping.

#### Scenario: Stop flag is checked between jobs
- **WHEN** the worker is mid-job (processing windows belonging to a single queue row) and the stop flag is set
- **THEN** the worker completes every window of the current job normally (cursor advances, queue row marks `done` atomically per the existing commit model)
- **AND** the worker does NOT claim the next pending queue row even if one is due
- **AND** the worker thread returns within the bounded shutdown budget

#### Scenario: Stop flag during idle exits promptly
- **WHEN** the worker is in the inter-poll sleep and the stop flag is set
- **THEN** the worker exits the loop on the next wake (or sooner if the sleep is interruptible)
- **AND** total exit latency from flag-set to worker-thread-return is under 2 seconds
