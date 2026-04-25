---
id: macos-mode-bits-hotfix
title: "Hotfix: macOS st_mode u16→u32 cast in fs_safety.rs"
status: complete
type: bugfix
owner: leela
reviewers: []
created: 2026-04-25
closes: "#79"
---

# Hotfix: macOS st_mode u16→u32 cast

## Problem

All macOS release builds (both `airgapped` and `online`, both `arm64` and `x86_64`) failed
during the v0.9.6 release workflow with:

```
error[E0308]: mismatched types
  --> src/core/fs_safety.rs:199:20
   |
   | mode_bits: stat.st_mode,
   |            ^^^^^^^^^^^^ expected `u32`, found `u16`
```

On macOS, `libc::stat::st_mode` is `u16`. On Linux it is `u32`. The `FileStatNoFollow`
struct declared `mode_bits: u32` without a platform-aware cast.

Because the build matrix uses `fail-fast: false` but the `release` job requires `needs:
build` to succeed completely, the release job never ran — no macOS assets were ever uploaded
to v0.9.6. Users on macOS received HTTP 404 when the installer tried to fetch the binary.
The asset naming in `install.sh` was correct throughout; this was a packaging gap caused
entirely by the compile failure.

## Fix

Single one-character change in `src/core/fs_safety.rs:199`:

```rust
// Before
mode_bits: stat.st_mode,

// After
mode_bits: stat.st_mode as u32,
```

The `as u32` widening cast is lossless on both platforms (u16 → u32 on macOS, u32 → u32
on Linux). The bit-mask operations in `FileStatNoFollow::is_directory()`,
`is_regular_file()`, and `is_symlink()` all operate on `u32` constants and remain correct.

## Non-Goals

- No behavioural change. No schema migration. No new surface area.
- Asset naming conventions are confirmed correct — no changes to `install.sh` or `release.yml`.

## Verification

`cargo check --no-default-features --features bundled` passes with zero errors on the host.
Full macOS cross-compile validation will be confirmed by the v0.9.7 CI run.
