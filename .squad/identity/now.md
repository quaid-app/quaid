updated_at: 2026-04-24T00:08:00Z
focus_area: vault-sync-engine post-L2 next-slice selection
active_issues: []
active_branch: spec/vault-sync-engine
---

# What We're Focused On

**Active change (vault-sync-engine):**

1. `vault-sync-engine` — Batch L2 closed; next slice not yet selected.
   Owner lane: Fry. Reviewers: Professor, Nibbler. Test lane: Scruffy.
   - L2 landed only the startup-only sentinel slice: recovery dir bootstrap, owned-sentinel startup recovery, and synthetic crash-mid-write proof
   - Keep writer-side sentinel creation, live recovery worker, and broader startup healing claims deferred
   - Pick the next truthful slice before widening any startup or write-through claims

**Completed in this branch:**
- Batch H — Phase 0-3 restore/remap safety helpers + fresh-connection full-hash activation
- Batch I — restore/remap orchestration + ownership recovery, including legacy write-gating and RCRT-only reopen
- Batch J — plain sync active-root reconcile path + CLI finalize truth fix
- Batch K1 — collection add/list plus truthful read-only gate
- Batch K2 — offline restore integrity closure with CLI finalize path
- Batch L1 — registry-startup scaffolding + restore-orphan startup recovery
- Batch L2 — startup-only sentinel recovery

**Explicitly deferred after L2:**
- Online restore handshake, IPC socket work, and the `17.5pp` / `17.5qq*` series that depend on IPC security design
- Broader MCP widening and remaining post-Tx-B / destructive restore surfaces beyond startup recovery
- Writer-side recovery sentinels (`12.x`), live/background recovery worker, and any claim that generic startup healing or remap reopen is already complete
- Post-landing coverage/docs/release/cleanup/issues agenda remains queued until the vault-sync branch reaches an appropriate stop point

**Gate:** No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes.
