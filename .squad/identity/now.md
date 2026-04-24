updated_at: 2026-04-24T04:19:19Z
focus_area: vault-sync-engine post-13.3 next-slice selection
active_issues: []
active_branch: spec/vault-sync-engine
---

# What We're Focused On

**Active change (vault-sync-engine):**

1. `vault-sync-engine` — Batch 13.3 closed (CLI parity only); next slice not yet selected.
    Owner lane: Fry. Reviewers: Professor, Nibbler. Test lane: Scruffy.
   - M1b-i closed the real write-interlock refusal seam for `17.5s2-s5`
   - M1b-ii closed Unix precondition/CAS hardening for `12.2`, `12.3`, `12.4a`, `17.5l-s`
   - M2a-prime closed the Windows platform gate for current vault-sync CLI handlers and truthfully narrowed `12.5` / `17.16a` to vault-byte entry points only
   - M2b-prime closed the same-slug within-process mutex + narrow mechanical write-through proof seam (`12.4`, narrow `17.5k`, `17.17e`)
   - M2c closed the explicit finalize-caller proof seam (`17.17b`) with a test-only invariant over production finalize helper call sites
   - M3a closed `2.4c` as a reconciler-specific wording/closure note: `ignore::WalkBuilder` enumeration with fd-relative revalidation and WARN-skip symlink behavior, not a generic `readdir` walk claim
   - N1 closed the MCP slug-routing truth seam only: slug-bearing MCP handlers now resolve collection-aware inputs first, page-referencing MCP outputs emit canonical `<collection>::<slug>` addresses, and ambiguity failures expose a stable machine-readable payload
   - 13.3 closed the CLI parity/output seam only: slug-bearing CLI commands now fail closed on ambiguous bare slugs, accept explicit `<collection>::<slug>` routing, and emit canonical page addresses on CLI outputs that reference pages, including single-page `embed`
   - `13.5` and `13.6` remain open; do not overclaim collection filters/defaults or a `brain_collections` tool
   - Pick the next truthful slice before widening into collection filters/defaults, IPC, watcher surfaces, or broader mutator coverage

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
- Batch M2c — explicit finalize-caller proof
- Batch M3a — reconciler symlink-walk wording closure
- Batch N1 — MCP slug-routing truth (`13.1`, `13.2`, `13.4` only)
- Batch 13.3 — CLI slug parity / canonical output closure

**Explicitly deferred after M1b:**
- Online restore handshake, IPC socket work, and the `17.5pp` / `17.5qq*` series that depend on IPC security design
- Broader MCP widening and remaining post-Tx-B / destructive restore surfaces beyond startup recovery
- Full `12.1`, any broader `12.4` claim beyond same-slug within-process mutex, `12.6*`, `12.7`, dedup `7.x`, and any full happy-path / dedup-echo-suppression closure beyond narrow `17.5k`
- Broader DB-only mutator coverage beyond the narrowed `12.5` / `17.16a` vault-byte closure
- Live/background recovery worker, IPC/live routing, and any claim that generic startup healing or remap reopen is already complete
- Post-landing coverage/docs/release/cleanup/issues agenda remains queued until the vault-sync branch reaches an appropriate stop point

**Gate:** 13.3 is closed. No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes.
