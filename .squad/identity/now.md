updated_at: 2026-04-24T17:44:00Z
focus_area: vault-sync-engine post-M2b next-slice selection
active_issues: []
active_branch: spec/vault-sync-engine
---

# What We're Focused On

**Active change (vault-sync-engine):**

1. `vault-sync-engine` — Batches M2a-prime and M2b-prime closed; next slice not yet selected.
   Owner lane: Fry. Reviewers: Professor, Nibbler. Test lane: Scruffy.
   - M1b-i closed the real write-interlock refusal seam for `17.5s2-s5`
   - M1b-ii closed Unix precondition/CAS hardening for `12.2`, `12.3`, `12.4a`, `17.5l-s`
   - M2a-prime closed the Windows platform gate for current vault-sync CLI handlers and truthfully narrowed `12.5` / `17.16a` to vault-byte entry points only
   - M2b-prime closed the same-slug within-process mutex + narrow mechanical write-through proof seam (`12.4`, narrow `17.5k`, `17.17e`)
   - Pick the next truthful slice before widening into routing, IPC, watcher surfaces, or broader mutator coverage

**Completed in this branch:**
- Batch H — Phase 0-3 restore/remap safety helpers + fresh-connection full-hash activation
- Batch I — restore/remap orchestration + ownership recovery, including legacy write-gating and RCRT-only reopen
- Batch J — plain sync active-root reconcile path + CLI finalize truth fix
- Batch K1 — collection add/list plus truthful read-only gate
- Batch K2 — offline restore integrity closure with CLI finalize path
- Batch L1 — registry-startup scaffolding + restore-orphan startup recovery
- Batch L2 — startup-only sentinel recovery
- Batch M1a — writer-side sentinel crash core
- Batch M1b-i — write-interlock closure
- Batch M1b-ii — Unix precondition and CAS hardening
- Batch M2a-prime — Windows platform gate + vault-byte read-only closure notes
- Batch M2b-prime — same-slug mutex + narrow mechanical ordering proof

**Explicitly deferred after M1b:**
- Online restore handshake, IPC socket work, and the `17.5pp` / `17.5qq*` series that depend on IPC security design
- Broader MCP widening and remaining post-Tx-B / destructive restore surfaces beyond startup recovery
- Full `12.1`, any broader `12.4` claim beyond same-slug within-process mutex, `12.6*`, `12.7`, dedup `7.x`, and any full happy-path / dedup-echo-suppression closure beyond narrow `17.5k`
- Broader DB-only mutator coverage beyond the narrowed `12.5` / `17.16a` vault-byte closure
- Live/background recovery worker, IPC/live routing, and any claim that generic startup healing or remap reopen is already complete
- Post-landing coverage/docs/release/cleanup/issues agenda remains queued until the vault-sync branch reaches an appropriate stop point

**Gate:** No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes.
