---
title: Install & Status
description: Current project status, supported install paths, and planned future distribution channels.
---

## Project Status

GigaBrain has completed **Phase 3** — all skills, benchmarks, CLI polish, and the full MCP tool surface are shipped. The current rollout is `v0.9.1`, which adds dual BGE-small release channels.

| Phase | Status | What ships |
| ----- | ------ | ---------- |
| **Sprint 0** — Repository scaffold | ✅ Complete | `Cargo.toml`, module stubs, `schema.sql`, skill stubs, CI workflows |
| **Phase 1** — Core storage + CLI | ✅ Complete | `gbrain init`, `import`, `get`, `put`, `search`, embeddings, hybrid search, MCP server — **v0.1.0** |
| **Phase 2** — Intelligence layer | ✅ Complete | `link`, `graph`, `check`, `gaps`, progressive retrieval, full MCP surface — **v0.2.0** |
| **Phase 3** — Skills + benchmarks + polish | ✅ Complete | All 8 skills production-ready, 16 MCP tools, `validate`, `call`, `pipe`, `skills doctor`, benchmark harnesses — **v0.9.1 dual-release prep** |

See the [Roadmap](/contributing/roadmap/) for ship gates and detailed scope.

---

## Install — Supported Now

### Build from source

The full Phase 3 binary compiles today. Build from source for all features.

**Requirements:** Rust stable toolchain. No other system dependencies — SQLite and sqlite-vec are bundled. The default source build is the **online** channel (downloads/caches BGE-small on first semantic use); build with `embedded-model` to produce the airgapped variant.

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain

# Online channel — default (downloads BGE-small weights on first semantic use)
cargo build --release
# Binary at: target/release/gbrain

# Airgapped channel — embeds BGE-small weights into the binary
cargo build --release --no-default-features --features bundled,embedded-model
```

#### Cross-compile for a fully static Linux binary

[`cross`](https://github.com/cross-rs/cross) requires a container runtime (Docker or Podman) to build inside target-specific containers.

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-musl      # Linux x86_64
cross build --release --target aarch64-unknown-linux-musl     # Linux ARM64
```

---

## Install — GitHub Releases

Pre-built binaries are available from GitHub Releases for `v0.9.1` in two BGE-small channels:

```bash
VERSION="v0.9.1"
PLATFORM="linux-x86_64"   # linux-x86_64 | linux-aarch64 | darwin-arm64 | darwin-x86_64
ASSET="gbrain-${PLATFORM}-airgapped"   # or: gbrain-${PLATFORM}-online
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/${ASSET}" -o "${ASSET}"
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/${ASSET}.sha256" -o "${ASSET}.sha256"
shasum -a 256 --check "${ASSET}.sha256"
# Option A: install for the current user
mkdir -p "${HOME}/.local/bin"
mv "${ASSET}" "${HOME}/.local/bin/gbrain"
chmod +x "${HOME}/.local/bin/gbrain"

# Option B: install system-wide (requires root)
sudo install -m 755 "${ASSET}" /usr/local/bin/gbrain
```

### One-command installer

The airgapped channel remains the default shell-installer path:

```bash
curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh
```

You can pin a version, switch to the online channel, or change the install directory:

```bash
GBRAIN_VERSION=v0.9.1 \
  curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh

GBRAIN_VERSION=v0.9.1 GBRAIN_CHANNEL=online \
  curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh

GBRAIN_VERSION=v0.9.1 GBRAIN_INSTALL_DIR="$HOME/.local/bin" \
  curl -fsSL https://raw.githubusercontent.com/macro88/gigabrain/main/scripts/install.sh | sh
```

> **Note:** The default install directory is `~/.local/bin`. System-wide paths like `/usr/local/bin`
> require `sudo` — the installer does not escalate privileges automatically.

The installer auto-detects your platform, chooses the airgapped or online release asset based on
`GBRAIN_CHANNEL`, verifies the SHA-256 checksum, runs `gbrain version`, and prints a `GBRAIN_DB`
shell-profile tip.

### npm global install (staged)

The npm package and postinstall downloader are implemented, but public npm publication is gated
until after the shell-installer test cycle and `NPM_TOKEN` is configured for release automation.

When that rollout opens, the install command will be:

```bash
npm install -g gbrain
```

The package downloads the **online** platform binary from GitHub Releases during `postinstall`
rather than bundling a native binary into the npm tarball. If you need the larger offline-ready
binary, use the default shell installer or download an `*-airgapped` asset from GitHub Releases.

---

## Coverage

CI runs `cargo check`, `cargo test`, and a coverage job on every push to `main` and pull request targeting `main`.

Coverage is generated with [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) and published as:
- A `coverage-report` artifact attached to each CI run (downloadable from the GitHub Actions run page), containing `lcov.info` (machine-readable) and `coverage-summary.txt` (text summary)
- A human-readable summary posted to the GitHub Actions job summary for that run
- An optional, non-blocking upload to Codecov when `CODECOV_TOKEN` is configured

Coverage is **informational** — the coverage job does not gate merges.

---

## What's next?

- [Quick Start](/guides/quick-start/) — five commands to a running brain
- [CLI Reference](/reference/cli/) — full subcommand reference
- [Roadmap](/contributing/roadmap/) — phased delivery plan and ship gates
- [Contributing](/contributing/contributing/) — how to propose and land a change
