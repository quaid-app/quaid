# Decision: Concurrent-open busy-timeout fix (benchmark lane)

**Date:** 2026-04-25  
**Author:** Kif  
**Affects:** `src/core/db.rs` → `open_connection`  
**CI run:** 24898484969 on head 18ac3d7

---

## What failed

`concurrent_readers_see_consistent_data` in `tests/concurrency_stress.rs` panicked on
every run.  Four reader threads each called `open_conn` → `db::open()` simultaneously
against an already-initialized on-disk database.  The panic was at line 31
(`db::open(path).unwrap_or_else(...)`) and line 320 (`handle.join().expect("reader thread
panicked")`).

## Root cause

`open_connection` in `src/core/db.rs` calls `Connection::open(path)` and then immediately
runs `execute_batch(schema.sql)`.  The schema batch begins with `PRAGMA journal_mode = WAL`
followed by multiple `CREATE TABLE IF NOT EXISTS` statements — DDL that requires a write
lock.  No busy timeout was set before this batch, so any thread that couldn't acquire the
write lock immediately received `SQLITE_BUSY` and the `?` propagation caused the panic.

The coordinator's fix in `tests/concurrency_stress.rs` added `conn.busy_timeout(1s)` to
the `open_conn` helper, but that call runs *after* `db::open()` returns — too late to
protect schema initialization.

## Why this is a runtime bug, not just a test fluke

Any two `gbrain` processes opening the same `brain.db` simultaneously (e.g. a background
`gbrain serve` and a foreground `gbrain query`) will hit the same failure.  The busy timeout
must be applied before DDL, not after.

## Fix

`src/core/db.rs`, `open_connection`:

```rust
let conn = Connection::open(path)?;
// Set busy timeout *before* schema DDL so concurrent opens don't race on the
// write lock required by the initial PRAGMA + CREATE TABLE IF NOT EXISTS batch.
conn.busy_timeout(Duration::from_secs(5))?;
conn.execute_batch(include_str!("../schema.sql"))?;
```

Five seconds is a conservative production ceiling.  The test helper's 1-second
post-open override is harmless and was left untouched.

## Verification

```
running 4 tests
test wal_compact_during_open_reader_both_succeed ... ok
test concurrent_readers_see_consistent_data ... ok
test parallel_occ_exactly_one_write_wins ... ok
test duplicate_ingest_from_two_threads_produces_one_row ... ok

test result: ok. 4 passed; 0 failed
```

`corpus_reality` (8/8 non-ignored) and `embedding_migration` (3/3) also pass.  
The two pre-existing lib failures (`open_rejects_nonexistent_parent_dir`,
`init_rejects_nonexistent_parent_directory`) reproduce identically on the unmodified head
and are out of scope for this lane.

## Benchmark lane status

All offline benchmark / stress gates are green.  No regressions introduced.
