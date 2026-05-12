## ADDED Requirements

### Requirement: `serve_sessions.session_type` accepts `daemon`, `serve_host`, `serve`, and `cli`

The existing `serve_sessions.session_type` column (extended additively at `src/core/db.rs:516-532` via `ensure_serve_session_columns`, currently default `'serve'`) SHALL accept four values: `'daemon'` | `'serve_host'` | `'serve'` | `'cli'`. The column expansion SHALL be additive — no `SCHEMA_VERSION` bump, no migration path — and SHALL preserve the existing default of `'serve'` so old binaries continue to round-trip safely. The `register_session` API SHALL accept a `SessionType` enum (`Daemon`, `ServeHost`, `Serve`, `Cli`) and persist the corresponding value into the column at registration time. All sweep/heartbeat/PK invariants previously specified SHALL continue to apply across all session types uniformly — a stale row of any type is swept identically.

#### Scenario: Session inserts carry the correct session_type
- **WHEN** `quaid daemon run`, `quaid serve` (first instance, promoted), `quaid serve` (subsequent instance, not promoted), and `quaid get` are each invoked
- **THEN** their `serve_sessions` rows carry `session_type` values `'daemon'`, `'serve_host'`, `'serve'`, and `'cli'` respectively
- **AND** any other value at INSERT time SHALL be rejected by application-layer validation (or by a future CHECK constraint, which the additive ALTER pattern allows)

#### Scenario: Stale rows of any session_type are swept identically
- **WHEN** the heartbeat sweeper runs and finds rows of all four session types past the 15s liveness threshold
- **THEN** all such rows are DELETEd (cascading to `collection_owners`)
- **AND** the sweep logic is session-type-agnostic

#### Scenario: Old binary safely reads new session_type values
- **WHEN** an older binary that recognizes only `'serve'` reads a `serve_sessions` table containing `'daemon'` and `'serve_host'` rows
- **THEN** the older binary's queries (which filter `WHERE session_type = 'serve'`) treat the new-typed rows as non-owners
- **AND** the older binary's `ensure_no_live_serve_owner` returns "no live serve owner" for collections actually owned by `daemon` or `serve_host`
- **AND** this is the safe partial-rollback fallback — the new binary's ownership predicates have been updated to accept all three runtime-host types (see the modification of `Live-serve coordination for restore/remap` below)

### Requirement: Atomic runtime-host promotion from `serve` to `serve_host`

The system SHALL provide a single-transaction `try_promote_to_serve_host(conn, session_id) -> bool` operation that atomically (a) sweeps stale `serve_sessions` rows, (b) checks whether a live `daemon` or live `serve_host` session exists for the database, (c) if neither exists, updates the caller's `serve_sessions.session_type` from `'serve'` to `'serve_host'` and returns `true`; otherwise returns `false`. The transaction SHALL use `BEGIN IMMEDIATE` (or an equivalent uniqueness guard) so concurrent invocations cannot both succeed. The first claimant becomes the unique `serve_host`; subsequent claimants stay `'serve'`.

#### Scenario: Concurrent promotion attempts elect exactly one winner
- **WHEN** no `daemon` is registered and two `quaid serve` processes call `try_promote_to_serve_host` concurrently
- **THEN** exactly one promotion succeeds and the caller observes `true`
- **AND** the other promotion observes `false` and remains `'serve'`
- **AND** the database contains exactly one row with `session_type = 'serve_host'`

#### Scenario: Promotion is refused while a daemon is live
- **WHEN** a `daemon` session is live and `quaid serve` calls `try_promote_to_serve_host`
- **THEN** the promotion observes `false` and the caller remains `'serve'`
- **AND** the database contains no `serve_host` row

#### Scenario: Promotion after stale daemon sweep
- **WHEN** a `daemon` row exists with `heartbeat_at < now() - 15s` and `quaid serve` calls `try_promote_to_serve_host`
- **THEN** the same transaction sweeps the stale daemon row, observes no live `daemon` or `serve_host`, and promotes the caller
- **AND** the caller observes `true`

### Requirement: Daemon registration fails when a live daemon already owns the database

The system SHALL enforce at most one live `daemon` session per database. The `quaid daemon run` startup path SHALL attempt to INSERT a `serve_sessions` row with `session_type = 'daemon'` inside a single SQLite transaction that first sweeps stale daemon rows (heartbeat past liveness threshold) and then attempts the INSERT. If the INSERT fails because another live daemon row exists, the new process SHALL exit non-zero with `DaemonAlreadyRunningError` and a message naming the existing daemon's PID and host. Concurrent `quaid daemon run` invocations SHALL not split-spawn watchers under any interleaving.

#### Scenario: Second daemon refuses to start
- **WHEN** a daemon is already running and a second `quaid daemon run` is invoked
- **THEN** the second invocation exits non-zero
- **AND** the error message names the existing daemon's PID
- **AND** no watcher or extraction-worker thread is spawned by the second process

#### Scenario: Crashed daemon is swept on retry
- **WHEN** a daemon dies via SIGKILL (leaving a stale row) and `quaid daemon run` is invoked after the liveness threshold passes
- **THEN** the new invocation sweeps the stale row in the same transaction as its INSERT
- **AND** the new invocation succeeds as the unique daemon

### Requirement: Daemon and serve_host are the watcher and supervised-duty owners; plain serve is transport-only

The runtime ownership rule SHALL be: a live `daemon` session is the unique owner of every supervised background duty for its database — the file watchers, the extraction worker, the idle-close timer, the janitor, the quarantine sweep, the full-hash audit, the RCRT (restore/remap continuation), and the embedding-queue drain. When no live `daemon` session exists, the unique live `serve_host` session SHALL own every duty listed above. Plain `serve` sessions SHALL register in `serve_sessions` for transport accounting and per-collection ownership leases (as already specified by the existing `Live-serve coordination` requirement), but SHALL NEVER spawn any of the supervised duties while a live `daemon` or `serve_host` exists. The "transport-only `serve` defers" rule preserves backwards compatibility at the user-facing CLI surface — a single `quaid serve` invocation still runs the full runtime — because internally that single invocation will succeed at the `serve_host` promotion when no other runtime host exists.

#### Scenario: Daemon-then-serve does not double-spawn any supervised duty
- **WHEN** `quaid daemon run` is running and a `quaid serve` invocation starts
- **THEN** only the daemon process owns the file watchers, extraction worker, idle-close timer, janitor, quarantine sweep, full-hash audit, RCRT, and embedding-queue drain
- **AND** the serve process opens only its MCP transport (and only when the MCP transport is enabled — stdio by default, HTTP with `--http`)

#### Scenario: Single bare-serve preserves full-runtime behavior
- **WHEN** `quaid serve` is invoked and no `daemon` or `serve_host` session exists
- **THEN** the serve process is promoted to `'serve_host'` and spawns every supervised duty, matching today's `start_serve_runtime` behavior end-to-end
- **AND** the user-facing experience of running `quaid serve` directly is unchanged

#### Scenario: Per-collection ownership leases still apply
- **WHEN** a `daemon` or `serve_host` session is live AND a `serve` session attaches
- **THEN** the runtime-host's session holds the `collection_owners` lease for each collection it watches
- **AND** the plain `serve` session does NOT attempt to claim those leases

## MODIFIED Requirements

### Requirement: Live-serve coordination for restore/remap (serve session ownership + rebind)

`quaid collection restore` and `quaid collection sync --remap-root` change `collections.root_path` and therefore invalidate the trusted `root_fd` and watcher state held by any running runtime-host process for that collection. The system SHALL implement explicit coordination between these commands and a live runtime-host process so a restore/remap NEVER leaves a runtime-host pinned to a stale root. Two mechanisms cover the two valid usage patterns:

**1. Runtime-host ownership — single-owner per-collection lease, role-agnostic.** Because AGENTS.md mandates `Single writer`, the ownership model SHALL enforce at most ONE live owner per collection via a transactional lease rather than an unconstrained sessions table. Schema: a `serve_sessions` table with columns `(session_id TEXT PRIMARY KEY, pid INTEGER NOT NULL, host TEXT NOT NULL, started_at TEXT NOT NULL, heartbeat_at TEXT NOT NULL, session_type TEXT NOT NULL DEFAULT 'serve')` — tracks heartbeat liveness across all session types — PLUS a `collection_owners` table `(collection_id INTEGER PRIMARY KEY REFERENCES collections(id) ON DELETE CASCADE, session_id TEXT NOT NULL REFERENCES serve_sessions(session_id) ON DELETE CASCADE, acquired_at TEXT NOT NULL)` that makes ownership exclusive per collection. The `PRIMARY KEY` on `collection_id` enforces one owner at a time.

At startup the runtime-host (either `quaid daemon run` or a `quaid serve` that has been promoted to `'serve_host'`) SHALL run ONE SQLite tx per collection that attempts an exclusive claim: (a) sweep stale `serve_sessions` rows (`heartbeat_at < now() - 15s`) AND cascade-delete their `collection_owners` rows; (b) INSERT its own `serve_sessions` row with a fresh UUID `session_id` and the appropriate `session_type` (`'daemon'` or `'serve_host'`); (c) attempt `INSERT INTO collection_owners (collection_id, session_id, acquired_at) VALUES (?, ?, now())`. If the INSERT fails with a PK conflict, the collection already has a live owner — the runtime-host SHALL refuse to attach the collection (log `runtime_refused_collection_owned collection=<N> owner_session=<S> owner_pid=<P> owner_type=<T>` at ERROR, continue with other collections that ARE free, exit non-zero if NO collections could be claimed). This prevents two runtime-hosts from watching the same collection simultaneously. Heartbeat refresh every 5 seconds (configurable via `QUAID_SERVE_HEARTBEAT_SECS`); a session is "live" when `heartbeat_at > now() - 15s` (three heartbeat intervals). At shutdown (SIGTERM, SIGINT, or clean exit) the runtime-host DELETEs its `serve_sessions` row which cascades to drop its `collection_owners` rows. On crash the row ages past the liveness threshold within 15s and the next runtime-host sweep reclaims it.

Plain `serve` sessions (transport-only) SHALL NOT attempt to claim `collection_owners`; only `daemon` and `serve_host` session types may hold collection ownership. `cli` sessions remain transport-irrelevant and never appear in `collection_owners`.

**1a. Command coordination with the owner lease.** Restore/remap/purge commands SHALL capture `expected_session_id` from `collection_owners` for the target collection (NOT from arbitrary `serve_sessions` rows). The ownership predicates `live_collection_owner`, `live_collection_owner_for_root_path`, `ensure_no_live_serve_owner`, `ensure_no_live_serve_owner_for_root_path`, and `acquire_owner_lease` SHALL treat **all three runtime-host session types** (`'daemon'`, `'serve_host'`, `'serve'`) as valid owning roles when joining `collection_owners` to `serve_sessions` — the existing implementation that filters `WHERE s.session_type = 'serve'` SHALL be widened to `WHERE s.session_type IN ('daemon','serve_host','serve')`. The third value (`'serve'`) is included only to keep older `quaid serve` rows (pre-`serve_host` promotion or written by an older binary) visible during partial-rollback windows; new code never assigns ownership to a `'serve'`-typed session. Because `collection_owners.collection_id` is a primary key, there is exactly ONE owning session per collection at any time — no "which runtime-host to coordinate with" ambiguity. If `collection_owners` has no row for the collection, there is no live owner; offline mode applies. The handshake helper re-reads `collection_owners` on every poll — if the owning session changes mid-handshake (e.g., the original owner crashed and a fresh runtime-host claimed the collection), the command aborts with `RuntimeOwnershipChangedError` and runs the abort-path resume-generation bump.

**1b. Startup contention handling.** A second runtime-host that starts while another is live observes a PK conflict on `collection_owners` and refuses the claim per (1). The contender logs the collision and exits non-zero if it cannot claim any collection; the user is directed to stop the running runtime-host via SIGTERM (`kill <pid>` or `quaid daemon stop` for an installed daemon). This is the single-writer enforcement — multiple runtime-host processes cannot silently split-memory the same collection.

**2. Command behavior (lease-based ownership resolution).** `quaid collection restore`, `quaid collection sync --remap-root`, AND `quaid collection remove --purge` SHALL, before mutating `collections.root_path`, `collections.state`, or cascading deletes:

- Read `collection_owners` for the target collection to resolve the owning session (NOT `serve_sessions` in the aggregate — the PK on `collection_owners.collection_id` guarantees exactly one owner).
- If `collection_owners` has NO row for the collection, there is no owner; proceed immediately (offline mode).
- If `collection_owners` has a row AND the referenced `serve_sessions.heartbeat_at` has aged past the 15s liveness threshold, the owner is stale. The command SHALL NOT silently adopt the stale lease; instead, it SHALL run the sweep (DELETE stale `serve_sessions` row → CASCADE drops the `collection_owners` row) in a tx, re-read `collection_owners` (now empty), and proceed as offline.
- If `collection_owners` has a row AND its referenced session is live (`heartbeat_at > now() - 15s`), the command SHALL select between two explicit behaviors:
 - **Default (no flag): refuse.** Return `RuntimeOwnsCollectionError` (renamed from the prior `ServeOwnsCollectionError`; the prior name MAY be retained as an alias for backwards compatibility in error matching) with a message naming the owning session's `pid`, `host`, and `session_type`, instructing the user to stop the runtime-host via SIGTERM (`kill <pid>` for a bare serve_host, `quaid daemon stop` for an installed daemon) and retry. No mutation occurs.
 - **`--online` flag: coordinate.** Perform the online rebinding handshake described next.

All three commands capture `expected_session_id = collection_owners.session_id` (the single owner) for use as the handshake key. No path reasons about "any live session" in `serve_sessions` — every ownership check goes through `collection_owners` joined to `serve_sessions` with the widened `session_type IN (...)` filter.

**3. Online rebinding handshake — lease-based ack bound to `(session_id, reload_generation)`.** When `--online` is passed and a live runtime-host session exists, the handshake SHALL use a release acknowledgement that is bound to the exact session and the exact generation bump the command is waiting for. A bare timestamp is NOT sufficient: a delayed write from an earlier timed-out handshake, or from a runtime-host instance that is racing shutdown/restart, could satisfy a later command even when the current owner has not released. The ack SHALL name both the releaser and the request it is releasing.

Schema additions on `collections` (beyond `reload_generation` and `watcher_released_at`):

- `watcher_released_session_id TEXT NULL` — session_id of the runtime-host that wrote the ack (must equal the session_id the command captured).
- `watcher_released_generation INTEGER NULL` — the generation value that was current when the ack was written (must equal the generation the command bumped to).

A handshake completes if and only if **all three** of these fields match the command's captured expectation:

- `watcher_released_session_id = <expected_session_id>`
- `watcher_released_generation = <cmd_reload_generation>`
- `watcher_released_at IS NOT NULL` (as a "has been set" signal; the timestamp itself is informational only, never a liveness test)

#### Scenario: Daemon-owned collection rejects offline restore with RuntimeOwnsCollectionError
- **WHEN** `quaid daemon run` holds the `collection_owners` lease for collection X, and `quaid collection restore X` is invoked without `--online`
- **THEN** the command exits with `RuntimeOwnsCollectionError`
- **AND** the message names the daemon's PID, host, and `session_type = 'daemon'`
- **AND** the message recommends `quaid daemon stop` rather than `kill <pid>` because the owner is an installed daemon

#### Scenario: serve_host-owned collection rejects offline restore with RuntimeOwnsCollectionError
- **WHEN** a `serve_host` (a bare `quaid serve` that was promoted) holds the `collection_owners` lease for collection X, and `quaid collection restore X` is invoked without `--online`
- **THEN** the command exits with `RuntimeOwnsCollectionError`
- **AND** the message names the runtime-host's PID, host, and `session_type = 'serve_host'`
- **AND** the message recommends SIGTERM via `kill <pid>` (no installed daemon to stop)

#### Scenario: Older `serve`-typed row is still treated as an owner during partial rollback
- **WHEN** an older binary wrote a `serve_sessions` row with `session_type = 'serve'` and holds `collection_owners` for collection X, and a new binary's `ensure_no_live_serve_owner` is called against collection X
- **THEN** the predicate joins `collection_owners` to `serve_sessions` with the widened `session_type IN ('daemon','serve_host','serve')` filter
- **AND** the predicate returns `RuntimeOwnsCollectionError` rather than treating the older row as a non-owner
- **AND** restore/remap commands therefore continue to refuse mutation, preserving the single-writer guarantee across mixed-binary scenarios
