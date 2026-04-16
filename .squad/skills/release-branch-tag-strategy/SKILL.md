# Skill: Release Branch / Tag Strategy

**Context:** How to create a real (non-fake) release branch and version tag for a Rust
binary project using GitHub Actions for cross-compilation.

---

## Pattern

### 1. Assess state before touching git

```bash
git status             # what's staged vs untracked
git log --oneline -5   # how far ahead of origin
git tag --list "v*"    # existing tags
```

Key questions:
- Is there a Cargo.toml version bump already?
- Is there any unstaged work that belongs in the release?
- Does a tag for this version already exist (locally or on remote)?

### 2. Create a named release branch from current HEAD

```bash
git checkout -b release/vX.Y.Z
```

**Why not main?** Release branches allow review + PR before merging to main.
The GitHub Actions `push.tags` trigger fires on any tag push regardless of branch,
so the branch choice doesn't affect CI.

### 3. Stage and commit all release-bound changes in one coherent commit

```bash
git add -A
git commit -m "feat: <description> — vX.Y.Z release lane

<body with files changed, gate summary, blocked items>"
```

Push the branch:
```bash
git push origin release/vX.Y.Z
```

### 4. Create an annotated tag (not lightweight)

```bash
git tag -a vX.Y.Z -m "vX.Y.Z — <one-liner>

<release description: what ships, what's staged, install snippet>"
```

Annotated tags (not lightweight) carry tagger identity + timestamp. GitHub shows
the tag message in the Releases view.

### 5. Push the tag to trigger CI

```bash
git push origin vX.Y.Z
```

This fires the release workflow. Verify within 30–60s:
```bash
gh run list --limit 5
```

Expected: Release workflow running + any staged-channel publish workflows completing
quickly with a skip notice.

---

## Prerelease flag convention

In `release.yml`:
```yaml
prerelease: ${{ contains(github.ref_name, '-') }}
```

- `v1.0.0` → full release
- `v0.9.0` → full release (pre-1.0 version number communicates test status)
- `v1.0.0-rc.1` → prerelease (contains `-`)

Use `-rc.N` / `-beta.N` suffixes when you want GitHub to mark the release as prerelease.

---

## Token-gated channel pattern

Staged channels (npm, crates.io, Homebrew) should use the "skip-if-absent, never-fail"
pattern in their publish workflows:

```yaml
- name: Skip publish when token is absent
  if: env.NPM_TOKEN == ''
  run: echo "::notice::NPM_TOKEN is not configured; skipping npm publish."

- name: Publish
  if: env.NPM_TOKEN != ''
  run: npm publish --access public
```

This lets the release workflow succeed on a fresh repo or a maintainer's fork without
any secrets configured, while the staged channel publish kicks in automatically once the
secret is added.

---

## Follow-up after tag push

1. Watch the CI build to completion (`gh run view <run-id>`)
2. Verify the GitHub Release was created with expected assets
3. Test the install script against the real release
4. Open a PR: `release/vX.Y.Z` → `main` to bring changes into the trunk
