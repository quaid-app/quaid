## Context

Fresh databases initialize with a default collection row, but write flows can still fail on first run when the write-target root is not configured to a writable path. This especially impacts conversation capture flows (`memory_add_turn`) where users expect immediate success in playground, MCP, and CLI-assisted onboarding.

The proposal sets a product requirement for sensible defaults: a first-run writable collection root at `~/.quaid/vault` without requiring manual collection bootstrap commands.

## Goals / Non-Goals

**Goals:**
- Ensure new databases have a usable, writable write-target root by default.
- Make first conversation-write operations succeed on clean installs without manual collection setup.
- Keep safety posture explicit: writes still route to a known local collection root owned by the current user.
- Preserve backward compatibility for already-configured databases.

**Non-Goals:**
- No migration that force-overwrites existing user-configured collection roots.
- No change to multi-collection routing semantics or ambiguity resolution rules.
- No changes to extraction policy, model download policy, or unrelated runtime bootstrap behavior.

## Decisions

1. Default root path for first-run write-target
- Decision: define the default collection root as `~/.quaid/vault` for new installs and unconfigured default-write-target states.
- Rationale: this is predictable, user-owned, and aligned with Quaid's home-directory convention.
- Alternatives considered:
  - `~/.quaid/default-vault`: clearer as a vault concept but less intuitive than `memory` for first-run users.
  - Keeping detached/empty root until explicit attach: safer by strictness but causes first-run failures and poor UX.

2. Initialization-time bootstrap over client-side workarounds
- Decision: establish the writable default in core initialization/first-run logic, not in individual clients (playground, wrappers).
- Rationale: one source of truth ensures consistent behavior across CLI, MCP, playground, and future clients.
- Alternatives considered:
  - Playground-only bootstrap: fixes one entry point but leaves MCP/CLI users exposed.
  - Lazy repair only inside `memory_add_turn`: narrower fix, but less transparent and repeats root-setup logic in request paths.

3. Compatibility behavior for existing databases
- Decision: only apply defaults when the write-target is missing or unconfigured; preserve valid existing roots.
- Rationale: prevents surprise relocation of storage and avoids breaking existing setups.
- Alternatives considered:
  - Hard migration for all DBs: too risky and intrusive.
  - No migration path at all: leaves old broken/unconfigured states unresolved.

## Risks / Trade-offs

- [Risk] Creating `~/.quaid/vault` in constrained environments may fail due to permissions.
  → Mitigation: return a clear configuration error and remediation guidance; keep existing explicit collection commands available.

- [Risk] Legacy DBs with unusual collection states may not fit a single bootstrap rule.
  → Mitigation: gate bootstrap on strict predicates (no writable root configured) and add integration tests for existing-state preservation.

- [Risk] Spec and docs drift if behavior changes but onboarding docs do not.
  → Mitigation: include docs updates and first-run acceptance tests in the task list.

## Migration Plan

- Apply behavior for newly initialized databases immediately.
- For existing databases, run a safe conditional bootstrap at open/startup only when write-target configuration is unusable.
- Do not modify databases that already have a writable configured write-target.
- Rollback strategy: disable conditional bootstrap path and revert to previous explicit setup requirements if regressions are found.

## Open Questions

- Should the conditional bootstrap run only at `quaid init`, or also at DB open when a legacy DB has an empty write-target root?
- Should bootstrap also normalize collection state (`detached` to `active`) for the default row when assigning `~/.quaid/vault`?
- What level of warning/telemetry should be emitted when automatic first-run bootstrap occurs?
