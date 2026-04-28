## PR #110 CI fix — accept stable rustfmt output

- **Date:** 2026-04-28
- **By:** Fry
- **Context:** PR #110 (`ops: harden main branch guardrails`) failed the `Check` job before any guardrail logic ran because `cargo fmt --check` on GitHub's current stable toolchain wanted formatting updates in existing Rust files outside the branch-guardrail surface.

### Decision

Treat the failure as repository formatting drift, not as a guardrails-workflow bug. Repair it by committing the current stable `cargo fmt --all` output on the PR branch, then re-running the existing validation (`cargo fmt --check`, `cargo check --all-targets`, `cargo test --verbose`, hook bootstrap simulation).

### Why

- The failure happened inside the existing `Cargo fmt` step in `.github/workflows/ci.yml`.
- Weakening or pinning around the fmt gate would change repository-wide CI policy for an unrelated ops PR.
- The rustfmt diff is mechanical and low risk; it keeps the repo aligned with the toolchain CI is already using.

### Scope notes

- No OpenSpec task truth changed for `protect-main-guardrails`.
- Main-branch protection and the direct-push guardrails remain intact.
