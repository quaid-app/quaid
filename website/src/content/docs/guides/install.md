---
title: Install & Status
description: Current project status, supported install paths, and planned future distribution channels.
---

## Project Status

GigaBrain has completed **Sprint 0** — the repository scaffold, CI, schema, and skill stubs are in place. Phase 1 (core storage, CLI, search, and MCP server) is in active development.

| Phase | Status | What ships |
| ----- | ------ | ---------- |
| **Sprint 0** — Repository scaffold | ✅ Complete | `Cargo.toml`, module stubs, `schema.sql`, skill stubs, CI workflows |
| **Phase 1** — Core storage + CLI | 🔨 In progress | `gbrain init`, `import`, `get`, `put`, `search`, embeddings, hybrid search, MCP server — ships as **v0.1.0** |
| **Phase 2** — Intelligence layer | ⏳ Planned | `link`, `graph`, `check`, `gaps`, progressive retrieval, full MCP surface — ships as **v0.2.0** |
| **Phase 3** — Polish + release | ⏳ Planned | Benchmark suite, full skill suite, release pipeline hardening — ships as **v1.0.0** |

See the [Roadmap](/contributing/roadmap/) for ship gates and detailed scope.

---

## Install — Supported Now

### Build from source

The scaffold compiles today. Full functionality ships in Phase 1.

**Requirements:** Rust stable toolchain. No other system dependencies — SQLite, sqlite-vec, and the embedding model are all bundled.

```bash
git clone https://github.com/macro88/gigabrain
cd gigabrain
cargo build --release
# Binary at: target/release/gbrain
```

#### Cross-compile for a fully static Linux binary

```bash
cargo install cross
cross build --release --target x86_64-unknown-linux-musl      # Linux x86_64
cross build --release --target aarch64-unknown-linux-musl     # Linux ARM64
```

---

## Install — Planned (Not Yet Available)

> The following install paths are **planned for future releases** and are not available today.

### GitHub Releases (planned for v0.1.0)

Once Phase 1 ships (v0.1.0), pre-built binaries will be available from GitHub Releases:

```bash
VERSION="v0.1.0"
PLATFORM="linux-x86_64"   # linux-x86_64 | linux-aarch64 | darwin-arm64 | darwin-x86_64
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}" -o "gbrain-${PLATFORM}"
curl -fsSL "https://github.com/macro88/gigabrain/releases/download/${VERSION}/gbrain-${PLATFORM}.sha256" -o "gbrain-${PLATFORM}.sha256"
shasum -a 256 --check "gbrain-${PLATFORM}.sha256"
mv "gbrain-${PLATFORM}" /usr/local/bin/gbrain && chmod +x /usr/local/bin/gbrain
```

### npm global install (deferred)

npm packaging is a deliberate follow-on. It is not in scope for the current release slice.

### One-command installer (deferred)

A `curl | sh` installer is a deliberate follow-on. Not in scope for the current release slice.

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
