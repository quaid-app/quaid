## Context

`scripts/install.sh` is a POSIX-compatible shell script that installs the `gbrain` binary
from GitHub Releases. As of `v0.9.1` it supports dual channels (airgapped/online),
checksums, and smoke-tests the binary. It does not touch shell profiles.

---

## Decisions

### 1. Auto-write is the default; opt-out is explicit

**Decision:** The installer writes to the shell profile by default. The user opts out with
`GBRAIN_NO_PROFILE=1` (env var, works with pipe-to-sh) or `--no-profile` (flag, works with
two-step download-then-run).

**Rationale:** The primary failure mode is silent: user installs, binary works in session,
then is unreachable on restart. Defaulting to profile writes eliminates the failure mode.
Opt-out is needed for CI environments that manage `$PATH` separately.

### 2. Detect profile via `$SHELL`, not fixed path

**Decision:** Use `$SHELL` to determine the preferred profile:
- `*/zsh` → `~/.zshrc`
- `*/bash` → `~/.bashrc` (Linux) or `~/.bash_profile` (Darwin, because `~/.bash_profile`
  is sourced by login shells on macOS)
- anything else → `~/.profile`

**Rationale:** Hardcoding `~/.zshrc` fails on bash users and vice versa. `$SHELL` is set
on all supported platforms and is more reliable than reading `/etc/shells`.

### 3. Idempotent writes — check before appending

**Decision:** Before appending a line, grep the target profile file for the exported
variable name (`PATH` / `GBRAIN_DB`). Skip the append if already present.

**Rationale:** Repeated installs (version upgrades) must not produce duplicate export lines.

### 4. Two-step install as a documented alternative, not default

**Decision:** The piped install (`curl ... | sh`) remains the primary recommended path.
The two-step pattern (`-o gbrain-install.sh && sh gbrain-install.sh`) is documented as
an explicit alternative for sandboxed environments. No default behavior change.

**Rationale:** Changing the primary path would break existing documentation and bookmarks.
The two-step workaround is narrowly useful; exposing it in the success output and docs is
sufficient.
