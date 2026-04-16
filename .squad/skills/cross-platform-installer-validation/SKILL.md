---
name: "cross-platform-installer-validation"
description: "Validate Unix shell installers and npm postinstall flows honestly from a Windows host."
domain: "validation"
confidence: "high"
source: "earned"
---

## Context
Use this when install or release validation must happen on Windows while the product's primary install paths target Unix shells or Linux/macOS npm environments.

## Patterns

### Normalize shell scripts first
- Check `.sh` files for CRLF before running them under WSL.
- Rewrite to LF if needed; otherwise `sh` can fail before any real installer logic executes.

### Keep temp state inside the repo
- Set a repo-scoped `TMPDIR` when invoking shell installers from WSL.
- Avoid `/tmp` and use repo-local scratch directories so validation is reproducible and policy-compliant.

### Use PATH-injected fakes for shell error branches
- For checksum or download failures, place a fake `curl` (or other dependency) earlier in `PATH`.
- This makes shell error-path validation deterministic without changing installer code.

### Use Node harnesses for postinstall branches
- When the host platform is unsupported, run `postinstall.js` through a small harness that overrides `process.platform` / `process.arch` and mocks `https.get`.
- This is good for graceful-failure validation (`ENOTFOUND`, 404s) when a real supported-platform npm lifecycle is unavailable.

### Separate environment blockers from product blockers
- Record host-only blockers (for example `EBADPLATFORM` on Windows or missing Node in WSL) separately from product blockers (missing release tags, package-name collisions).
- Leave tasks open if the true end-to-end path was not executed.
