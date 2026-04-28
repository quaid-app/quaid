---
name: "stable-rustfmt-drift"
description: "Handle CI failures caused by stable rustfmt drift without weakening repo guardrails."
domain: "ci"
confidence: "high"
source: "observed"
tools:
  - name: "cargo fmt"
    description: "Reproduce and repair formatting drift against the current stable toolchain."
    when: "The CI gate fails in cargo fmt --check with purely mechanical diffs."
---

## Context

Use this when a PR fails the formatting gate even though the feature branch did not intentionally edit Rust logic. The goal is to confirm the failure is mechanical drift, fix it with the minimum safe change, and keep the existing CI policy intact.

## Patterns

- Reproduce the failure locally with `cargo fmt --all -- --check` on the exact PR head.
- Inspect the diff to confirm it is formatting-only and does not widen behavior.
- Prefer `cargo fmt --all` over weakening the workflow or pinning around stale formatting unless the repo has an explicit pinned rustfmt policy already.
- Re-run the relevant validation after the reformat: `cargo fmt --all -- --check`, build/test commands already enforced by CI, plus any feature-specific smoke checks touched by the PR.

## Examples

- PR #110 (`ops: harden main branch guardrails`) failed `Check` because stable rustfmt reformatted existing files in `src/commands/`, `src/core/`, and `tests/command_surface_coverage.rs`; the safe fix was a formatting-only commit plus normal validation.

## Anti-Patterns

- Do not relax or remove the fmt gate just to get one PR green.
- Do not describe a rustfmt-only failure as a product or workflow logic regression.
- Do not mix unrelated hand edits into the formatting repair commit.
