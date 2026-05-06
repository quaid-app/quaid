## ADDED Requirements

### Requirement: All writers of conversation day-files acquire the session lock

Any code path that mutates a conversation day-file at `<vault>/[<namespace>/]conversations/<YYYY-MM-DD>/<session-id>.md` — including `memory_add_turn`, `memory_close_session`, the CLI `extract --force` cursor-reset path, and any future admin tool — SHALL first acquire the same per-session in-process mutex (`session_lock`) and on-disk advisory `SessionFileLock` used by `memory_add_turn`'s `append_turn` implementation. No writer is permitted to bypass this discipline. This requirement extends the existing same-session serialization guarantee to cover **all** writers, not only turn appends, so that admin or maintenance writes cannot clobber concurrent appends from a running MCP server.

#### Scenario: Cursor reset serializes against a concurrent turn append

- **WHEN** `quaid extract s1 --force` is invoked while an MCP `memory_add_turn` for the same session is in flight against the same day-file
- **THEN** the two writers serialize on the session lock, the appended turn block is preserved on disk, and the cursor reset's frontmatter mutation does not regress past the appended turn's ordinal

#### Scenario: Lock-respecting rewrite preserves concurrent append

- **WHEN** writer A acquires the session lock and parses a day-file containing turns `1..N`, then writer B (a concurrent appender) waits on the lock until A releases it
- **THEN** writer B's append observes A's mutated frontmatter and produces turn `N+1`, with neither writer's mutation lost

#### Scenario: Bypass is prevented by code review and verified by tests

- **WHEN** the cursor-reset path is exercised by tests that hold the session lock from a separate worker
- **THEN** the cursor-reset path blocks until the lock is released, verifying that it actually contends on the same primitive

### Requirement: Cursor reset never writes while another writer holds the session lock

The cursor-reset path used by `extract --force` SHALL NOT write to a day-file while another writer holds the on-disk `SessionFileLock` for that session. The implementation SHALL satisfy this either by acquiring the lock with a bounded wait before writing or by exiting non-zero with an error message naming the contended day-file. Either policy MAY be chosen, but the no-write-while-locked invariant SHALL hold unconditionally.

#### Scenario: Forced reset with a held lock surfaces a clear error

- **WHEN** `quaid extract s1 --force` runs while an external process holds the on-disk `SessionFileLock` for `s1`
- **THEN** the command either blocks until the lock is released or exits non-zero with an error message naming the contended day-file; in no case does it write while the lock is held by another process
