---
change: macos-mode-bits-hotfix
---

# Tasks

## T1 — Fix st_mode cast in fs_safety.rs ✅

**File:** `src/core/fs_safety.rs:199`  
**Change:** `mode_bits: stat.st_mode` → `mode_bits: stat.st_mode as u32`  
**Status:** Done

## T2 — Bump Cargo.toml to 0.9.7 ✅

**File:** `Cargo.toml`  
**Change:** `version = "0.9.6"` → `version = "0.9.7"`  
**Status:** Done

## T3 — Release branch, tag, and PR ✅

Create `release/v0.9.7`, push tag `v0.9.7`, open PR to main.  
**Status:** Done (see routing memo)
