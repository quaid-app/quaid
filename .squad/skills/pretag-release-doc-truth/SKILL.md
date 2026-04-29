---
name: pretag-release-doc-truth
version: 1.0
author: amy
last_updated: 2026-04-29T21:29:11.071+08:00
---

# Pre-tag release doc truth

Use this when a release branch or version bump exists before the matching GitHub tag and release assets are actually published.

## When to apply

- `Cargo.toml` already shows the next version, but GitHub Releases still serves the older public tag.
- README, getting-started docs, or docs-site install pages show literal `vX.Y.Z` download commands for a tag that does not exist yet.
- Review feedback says the docs are "stale" or "blocking release," but the correct fix is truthfulness, not pretending the tag already shipped.

## Pattern

### 1. Split branch state from published state

- In status prose, say the branch **prepares** or **targets** the upcoming release.
- In install prose, say GitHub Releases and `install.sh` use the **latest published tag**.
- Do not collapse those two states into one sentence.

### 1a. Check workflow-enforced version gates

- If the release workflow verifies the manifest version against the tag, audit that manifest (`Cargo.toml`, `package.json`, etc.) as part of pretag truth work.
- A branch can be docs-ready and test-green but still be non-shippable if the manifest still points at the last public version.

### 2. Use placeholders in release-asset commands

Prefer:

```bash
VERSION="<published-tag>"
```

or:

```bash
QUAID_VERSION=<published-tag> sh
```

Then add a short note explaining that readers should replace the placeholder with a real published tag, or build from source if they need the unreleased branch behavior.

### 3. Name the branch-only feature delta

If the tool count or install surface did not change, say so directly. Example: "This release keeps the 17-tool MCP surface and adds background embedding drain plus queue reporting."
Also check roadmap/deferred tables for stale "not yet implemented" language; they often drift separately from README/install docs after a follow-on slice lands.
When a previously deferred CLI/admin command ships, update both the roadmap's deferred table and the user docs that teach the old workflow. A merged feature like `--write-quaid-id` / `migrate-uuids` can still block release if public docs leave it stranded in "future work."

## Audit checklist

- [ ] Manifest version (`Cargo.toml`, `package.json`, etc.) matches the intended tag once the release is actually ready to cut
- [ ] `README.md`
- [ ] `docs/getting-started.md`
- [ ] `website/src/content/docs/tutorials/install.mdx`
- [ ] `docs/roadmap.md` if getting-started or README links readers there for release status

## Anti-patterns

- Replacing the old version number with the upcoming tag in download commands before the tag exists
- Saying "current release is vX.Y.Z" when only the branch is at that version
- Leaving install instructions with a stale prior tag after the branch has already moved to the next release lane
