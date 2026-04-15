---
name: "Rust Coverage in GitHub Actions"
description: "Deploy code coverage reporting on Rust projects using cargo-llvm-cov, Codecov, and GitHub Pages. Supports cross-platform CI with PR integration."
domain: "testing, ci-cd, reporting"
confidence: "high"
source: "investigation (2026-04-15), Rust ecosystem best practices"
tools:
  - name: "cargo-llvm-cov"
    description: "LLVM-based coverage for Rust; generates LCOV, HTML, Cobertura JSON"
    when: "primary coverage collection for cross-platform projects"
  - name: "Codecov"
    description: "Free coverage reporting service; auto PR comments, badges, threshold gates"
    when: "badge, PR feedback, historical dashboard for public repos"
  - name: "peaceiris/actions-gh-pages"
    description: "GitHub Actions for deploying HTML to GitHub Pages (gh-pages branch)"
    when: "secondary self-hosted dashboard (optional, complements Codecov)"
---

## Context

Rust projects on GitHub need coverage reporting that:
- Runs on every push to main (or PR)
- Works across platforms (macOS arm64/x86_64, Linux musl, Windows)
- Reports to multiple surfaces: README badge, PR comments, status checks, dashboards
- Remains free and low-friction for public repos

**Constraints:**
- Most Rust projects test on Linux; coverage tooling must also work on macOS (for releases)
- GitHub Actions cache and artifact uploads impact CI time
- Codecov API availability is not guaranteed (use `fail_ci_if_error: false`)

**Related decisions:**
- GitHub Actions CI must run on multi-platform matrix (macOS, Linux)
- Release workflow cross-compiles to macOS arm64/x86_64 and Linux musl targets

## Patterns

### Pattern 1: Primary Coverage with cargo-llvm-cov + Codecov

**When to use:**
- New Rust projects or upgrading existing coverage
- Need to support macOS and Linux builders
- Want PR comment integration and historical trends

**Implementation:**

1. **Install and run coverage in CI job:**
   ```yaml
   - name: Install LLVM tools (for coverage)
     run: rustup component add llvm-tools-preview

   - name: Install cargo-llvm-cov
     uses: taiki-e/install-action@cargo-llvm-cov

   - name: Generate coverage LCOV
     run: cargo llvm-cov --lcov --output-path lcov.info --summary-only --fail-under-lines 80

   - name: Upload to Codecov (free for public)
     uses: codecov/codecov-action@v4
     with:
       files: ./lcov.info
       flags: unittest
       fail_ci_if_error: false  # Don't block on Codecov outage
   ```

2. **Add badge to README:**
   ```markdown
   [![codecov](https://codecov.io/gh/USER/REPO/branch/main/graph/badge.svg)](https://codecov.io/gh/USER/REPO)
   ```

3. **Sign up for Codecov (free for public repos):**
   - https://codecov.io
   - Authorize GitHub app
   - PR comments and dashboard are automatic

**Trade-offs:**
- ✅ Cross-platform, includes doc tests, historically stable
- ✅ Codecov free tier: 1 year history, unlimited repos, PR integration
- ⚠️ Requires `llvm-tools-preview` component (~100MB download)
- ⚠️ Codecov is external service (though it never blocks CI with `fail_ci_if_error: false`)

**Coverage thresholds (gates):**
- Phase 1/2: aspirational ≥70% (doesn't fail)
- Phase 3: enforced via Codecov checks: fail if delta >2%

---

### Pattern 2: Secondary GitHub Pages Dashboard

**When to use:**
- Want self-hosted, persistent historical HTML dashboard
- Already publishing other docs to GitHub Pages
- Don't want to rely solely on external Codecov service

**Implementation:**

1. **Generate HTML coverage in CI:**
   ```yaml
   - name: Generate coverage HTML
     run: cargo llvm-cov --html

   - name: Deploy to GitHub Pages
     uses: peaceiris/actions-gh-pages@v4
     with:
       github_token: ${{ secrets.GITHUB_TOKEN }}
       publish_dir: ./target/llvm-cov/html
       destination_dir: coverage  # Creates /coverage/ under Pages root
   ```

2. **Enable GitHub Pages in repo settings:**
   - Settings > Pages > Source: Deploy from branch
   - Select `gh-pages` branch

3. **Access at:**
   - `https://USER.github.io/REPO/coverage/`

**Trade-offs:**
- ✅ Self-hosted, no external service for dashboard
- ✅ Full HTML with drill-down per file/line
- ✅ Historical via git log of `gh-pages` branch
- ⚠️ Requires `gh-pages` branch setup
- ⚠️ Refresh only on successful push (async)
- ⚠️ Cannot gate merges on Pages coverage (PR checks only via Codecov)

**Combination:**
- Run Codecov (primary, PR gates + badge) and Pages (secondary, self-hosted HTML) in parallel
- Both read from same LCOV file; zero conflict

---

### Pattern 3: Fallback — cargo-tarpaulin

**When to use:**
- LLVM tools cause build failures on macOS or Windows runners
- Team decides to test Linux-only (simplified CI)
- Lighter CI footprint is hard requirement

**Implementation:**
```yaml
- name: Install tarpaulin
  run: cargo install cargo-tarpaulin

- name: Generate LCOV
  run: cargo tarpaulin --out Lcov --output-dir . --timeout 120

- name: Upload to Codecov
  uses: codecov/codecov-action@v4
  with:
    files: ./lcov.info
```

**Trade-offs:**
- ✅ Simpler install, lighter CI footprint (~30s vs 45s)
- ❌ Linux-only (ptrace dependency)
- ❌ Experimental macOS support
- ❌ Poor doc test coverage

---

## Examples

### Full CI Job: Primary + Secondary

```yaml
name: Coverage

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  coverage:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: llvm-tools-preview

      - name: Cache cargo
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}

      - uses: taiki-e/install-action@cargo-llvm-cov

      - name: Run tests with coverage
        run: cargo llvm-cov --lcov --output-path lcov.info --summary-only --fail-under-lines 80

      # Primary: Upload to Codecov (PR comments + badge)
      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: ./lcov.info
          flags: unittest
          fail_ci_if_error: false

      # Secondary (optional): Deploy HTML to GitHub Pages
      - name: Generate coverage HTML
        run: cargo llvm-cov --html

      - name: Deploy coverage dashboard to GitHub Pages
        uses: peaceiris/actions-gh-pages@v4
        if: github.ref == 'refs/heads/main'
        with:
          github_token: ${{ secrets.GITHUB_TOKEN }}
          publish_dir: ./target/llvm-cov/html
          destination_dir: coverage
```

---

## Anti-Patterns

❌ **Don't use grcov as primary.**  
grcov is a post-processor for LLVM coverage data. It overlaps with cargo-llvm-cov but requires manual `.profraw` file handling. Use only if you need custom coverage logic (e.g., filtering specific source paths). Most teams should use cargo-llvm-cov directly.

❌ **Don't gate main branch on Codecov availability.**  
Use `fail_ci_if_error: false` in codecov/codecov-action. Codecov outages shouldn't block deployments.

❌ **Don't mix tarpaulin and llvm-cov in same CI job without branching.**  
Each produces different LCOV coverage metrics due to instrumentation differences. Pick one per OS/job.

❌ **Don't rely solely on GitHub Pages for coverage gating.**  
GitHub Pages HTML is generated after CI completes. Use Codecov status checks (via API) to gate PRs in real-time. Pages is a _view_, not a gate.

❌ **Don't forget `llvm-tools-preview` component.**  
If you skip `rustup component add llvm-tools-preview`, cargo-llvm-cov will silently fall back to pseudo-coverage (hash-based) instead of real LLVM instrumentation.

---

## Metrics / Success Criteria

✅ Coverage reports appear in:
- README: badge updates on each push
- PR: Codecov auto-comments within 1 min
- Status checks: PR blocked if delta > threshold (if configured)
- Dashboard: codecov.io shows historical trend
- (Optional) Pages: self-hosted HTML at `/coverage/` refreshes on main push

✅ CI time increase: ~15s (install) + 30s (coverage) = ~45s per push

✅ Zero maintenance after setup (Codecov runs automatically on token-free public repos)

