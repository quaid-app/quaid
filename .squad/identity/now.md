updated_at: 2026-04-25T00:25:00Z
focus_area: vault-sync-engine post-9.10 next-slice selection
active_issues: []
active_branch: spec/vault-sync-engine
---

# What We're Focused On

**Active change (vault-sync-engine):**

1. `vault-sync-engine` — Batch 9.10 closed (collection ignore CLI truth only); next slice not yet selected.
    Owner lane: Fry. Reviewers: Professor, Nibbler. Test lane: Scruffy.
   - M1b-i closed the real write-interlock refusal seam for `17.5s2-s5`
   - M1b-ii closed Unix precondition/CAS hardening for `12.2`, `12.3`, `12.4a`, `17.5l-s`
   - M2a-prime closed the Windows platform gate for current vault-sync CLI handlers and truthfully narrowed `12.5` / `17.16a` to vault-byte entry points only
   - M2b-prime closed the same-slug within-process mutex + narrow mechanical write-through proof seam (`12.4`, narrow `17.5k`, `17.17e`)
   - M2c closed the explicit finalize-caller proof seam (`17.17b`) with a test-only invariant over production finalize helper call sites
   - M3a closed `2.4c` as a reconciler-specific wording/closure note: `ignore::WalkBuilder` enumeration with fd-relative revalidation and WARN-skip symlink behavior, not a generic `readdir` walk claim
   - N1 closed the MCP slug-routing truth seam only: slug-bearing MCP handlers now resolve collection-aware inputs first, page-referencing MCP outputs emit canonical `<collection>::<slug>` addresses, and ambiguity failures expose a stable machine-readable payload
   - 13.3 closed the CLI parity/output seam only: slug-bearing CLI commands now fail closed on ambiguous bare slugs, accept explicit `<collection>::<slug>` routing, and emit canonical page addresses on CLI outputs that reference pages, including single-page `embed`
    - 13.6 closed the read-only `brain_collections` MCP seam only: frozen 13-field output, truthful recovery/blocker/restore semantics, and parse-error-only `ignore_parse_errors` surfacing; stable-absence refusal surfacing remains deferred to `17.5aa5`
    - 13.5 closed the MCP read-filter seam only: `brain_search`, `brain_query`, and `brain_list` accept an optional `collection` filter, default to the sole active collection when exactly one exists, otherwise the write-target collection, and keep `brain_query depth="auto"` expansion fenced to that collection
    - 9.10 / 9.11 closed the collection-ignore CLI seam only: `gbrain collection ignore add|remove|list|clear --confirm` now uses dry-run-first validation, explicit clear semantics, mirror refresh via ignore helpers, active-root reconcile proofs, and the current collection CLI surface emits stable JSON success payloads with non-zero error exits
    - Pick the next truthful slice before widening into watcher-driven ignore reload, broader ignore diagnostics, IPC, or broader mutator coverage

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
- Batch 13.5 — MCP collection-filter truth closure
- Batch 13.6 — `brain_collections` MCP schema/truth closure (`13.6`, `17.5ddd` only)
- Batch 9.10 / 9.11 — collection ignore CLI + success-summary truth closure

**Explicitly deferred after M1b:**
- Online restore handshake, IPC socket work, and the `17.5pp` / `17.5qq*` series that depend on IPC security design
- Broader MCP widening and remaining post-Tx-B / destructive restore surfaces beyond startup recovery
- Full `12.1`, any broader `12.4` claim beyond same-slug within-process mutex, `12.6*`, `12.7`, dedup `7.x`, and any full happy-path / dedup-echo-suppression closure beyond narrow `17.5k`
- Broader DB-only mutator coverage beyond the narrowed `12.5` / `17.16a` vault-byte closure
- Live/background recovery worker, IPC/live routing, and any claim that generic startup healing or remap reopen is already complete
- Post-landing coverage/docs/release/cleanup/issues agenda remains queued until the vault-sync branch reaches an appropriate stop point

**Gate:** 13.5, 13.6, and 9.10 / 9.11 are closed. No next vault-sync slice is active yet; require a fresh scoped gate before implementation resumes.
