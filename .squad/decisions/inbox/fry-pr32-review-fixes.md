# Decision: npm bin wrapper pattern for gbrain npm package

**Author:** Fry  
**Date:** 2026-04-16  
**Context:** PR #32 review — Copilot reviewer flagged that `bin/gbrain` didn't exist at npm install time, causing bin-linking failures.

## Decision

Ship a committed POSIX shell wrapper at `packages/gbrain-npm/bin/gbrain` that:
1. Checks for `gbrain.bin` (the native binary downloaded by postinstall.js)
2. If found, `exec`s it with all arguments forwarded
3. If not found, prints a clear manual-install fallback message to stderr and exits 1

The postinstall.js script now writes the downloaded binary to `bin/gbrain.bin` (not `bin/gbrain`), so the wrapper is never overwritten. `.gitignore` tracks `gbrain.bin` and `gbrain.download`; the wrapper itself is version-controlled.

## Rationale

- npm creates bin symlinks before postinstall runs — the target file must exist at pack time
- The wrapper gracefully handles postinstall skip (unsupported platform, network failure, CI)
- Users get an actionable error instead of "command not found"

## Scope

This pattern applies to the `packages/gbrain-npm/` package only. No impact on the shell installer or Cargo binary.
