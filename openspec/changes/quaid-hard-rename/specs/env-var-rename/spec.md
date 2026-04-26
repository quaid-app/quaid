# Env Var Rename Spec

**Change:** All environment variables renamed from the legacy prefix to `QUAID_*`.

## Current variable names (post-rename)

| New variable | Used in |
|--------------|---------|
| `QUAID_DB` | `src/main.rs`, docs |
| `QUAID_MODEL` | `src/main.rs`, docs |
| `QUAID_CHANNEL` | `scripts/install.sh`, docs |
| `QUAID_INSTALL_DIR` | `scripts/install.sh` |
| `QUAID_VERSION` | `scripts/install.sh` |
| `QUAID_NO_PROFILE` | `scripts/install.sh` |
| `QUAID_RELEASE_API_URL` | `scripts/install.sh` |
| `QUAID_RELEASE_BASE_URL` | `scripts/install.sh` |

## Invariants

1. No legacy env var prefix appears in any source file, script, workflow, or documentation in the final implementation.
2. The `clap` `env()` attributes in `src/main.rs` must reference `QUAID_DB` and `QUAID_MODEL`.
3. `scripts/install.sh` must reference only `QUAID_*` variables.
4. All docs and SKILL.md examples must use `QUAID_*` names.
5. No forwarding shim reads legacy env vars and exports as `QUAID_*`.

## Validation

- A search for the legacy env var prefix across all text-type files returns zero matches (excluding `.squad/` history files).
