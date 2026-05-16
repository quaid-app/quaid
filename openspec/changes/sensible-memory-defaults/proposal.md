## Why

Fresh installs currently require users to manually bootstrap a writable collection root before conversation tools can write turns. This causes first-run failures in common flows (including playground and MCP `memory_add_turn`) and creates avoidable setup friction.

## What Changes

- Define sensible first-run collection defaults during database initialization so conversation and write flows work without manual collection setup.
- Set the default write-target collection root to `~/.quaid/vault` (resolved to the current user's home directory) when no writable root is configured.
- Ensure first-run defaults preserve existing safety expectations (explicit writable target, local user-owned path, no hidden remote behavior).
- Document and test the first-run behavior so CLI, MCP, and playground flows consistently succeed on a clean install.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `collections`: Change first-run default collection behavior so a writable write-target root is automatically available at `~/.quaid/vault`.
- `conversation-turn-capture`: Require `memory_add_turn` and related conversation-write tools to succeed on a fresh initialized database without prior manual collection bootstrap.

## Impact

- Affected areas: DB initialization/default collection setup, collection metadata defaults, conversation turn writer preconditions, and first-run docs.
- Affected surfaces: CLI (`quaid init`), MCP conversation tools, playground first-run UX.
- Compatibility: Existing configured databases should keep their current collection roots; defaults apply to new DBs or unconfigured default write-target states.
