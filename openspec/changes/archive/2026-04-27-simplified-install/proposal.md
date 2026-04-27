---
id: simplified-install
title: "Simplified Install: npm global + curl one-liner"
status: in-progress
type: feature
owner: fry
reviewers: [leela, kif]
created: 2026-04-16
depends_on: p3-skills-benchmarks
---

# Simplified Install: npm global + curl one-liner

## Why

Quaid needs a simpler install story for the upcoming `v0.9.0` test release. Pre-built
binaries for macOS ARM/x86 and Linux x86_64/ARM64 are already part of the release pipeline,
but the current README and website docs still make users assemble the install manually and
still describe both lightweight install paths as deferred:

- `npm install -g quaid`
- One-command curl installer

The current GitHub Releases install workflow requires the user to know their platform string,
construct a URL, download two files (binary + checksum), run a checksum command, move the
binary, and set permissions. This is six steps too many for a tool that should feel as quick
to grab as any other CLI.

Two audiences benefit immediately:

1. **JS-toolchain users** — developers who already have Node.js installed reach for
   `npm install -g` instinctively. They should not need to discover GitHub Releases.

2. **Blog post / tutorial readers** — any article about Quaid will lead with a single
   install command. A one-command curl installer is the standard for this class of tool
   (Homebrew, rustup, bun, etc.).

Both install paths still matter, but the rollout plan has changed: ship and test the shell
installer first in `v0.9.0`, while landing the npm package scaffolding and publish automation
in a way that does not fail builds when `NPM_TOKEN` is absent. Public npm publication can follow
once the shell path is proven.

## What Changes

### 1. curl one-liner installer (`scripts/install.sh`)

A POSIX-compatible shell script that auto-detects the host platform, downloads the correct
release binary from GitHub Releases, verifies its SHA-256 checksum, and installs to the
user's local bin directory — no sudo required by default.

Invocation:

```sh
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh | sh
```

Or with version and directory overrides:

```sh
curl -fsSL https://raw.githubusercontent.com/quaid-app/quaid/main/scripts/install.sh \
  | QUAID_VERSION=v0.9.0 QUAID_INSTALL_DIR="$HOME/.local/bin" sh
```

After installing, the script prints a tip suggesting the user set `QUAID_DB` in their
shell profile. The `QUAID_DB` env var is already supported by the binary — without it,
the default database path is `./memory.db` (current working directory), which is not useful
for a globally-installed CLI. The installer does **not** modify any shell files
automatically; it only prints the suggestion:

```
Tip: Set QUAID_DB to avoid passing --db on every command:
  echo 'export QUAID_DB="$HOME/memory.db"' >> ~/.zshrc
```

### 2. npm global package scaffolding (`packages/quaid-npm/`)

A zero-code npm package named `quaid` that uses a `postinstall` Node.js script to download
the correct platform binary from GitHub Releases. The npm tarball contains only the wrapper
script — the 90MB binary is never bundled inside it.

This change lands the package layout, postinstall logic, and publish workflow, but keeps the
public rollout gated behind successful `v0.9.0` shell-installer testing and an explicitly
configured `NPM_TOKEN` secret.

### 3. Docs updates

- **README.md** — Update install table to show the curl installer as available for the
  `v0.9.0` test release and the npm package as staged but not yet public.
- **website/src/content/docs/guides/install.md** — Replace the deferred curl section with
  live shell-installer instructions and explain the staged npm rollout.
- **docs/getting-started.md** — Update install options and rollout messaging to match the
  shell-first test release.

## Non-Goals

- **Homebrew formula** — Valid follow-on but involves its own tap/PR process. Out of scope.
- **Windows installer / winget / .msi** — Separate proposal when Windows is a supported
  target.
- **Bundling the binary in the npm tarball** — Would bloat npm installs to ~90MB and
  require platform-specific npm packages. Rejected in favour of postinstall download.
- **Publishing to crates.io** — Quaid is an application binary, not a library.
  `cargo install` from source is already supported.

## Impact

- Users can go from zero to a working `quaid` binary in one command during the `v0.9.0`
  test cycle.
- No Rust toolchain is required for end users using GitHub Releases or the shell installer.
- Install script can be linked directly from the README quick-start.
- npm packaging and publish automation are prepared without making `NPM_TOKEN` a hard
  requirement for the release workflow.
