# Source Seam Invariants

Use this when production behavior is intentionally narrow but the most valuable proof is "these exact production call sites / branches still exist."

## When to use

- Cross-platform logic is hard to execute directly on the current host.
- A helper contract is only credible if every production caller keeps a required argument, guard, or filter.
- Refactors could silently widen or bypass a safety seam even while behavior-style tests stay green.

## Pattern

1. Read the production source file and strip the `#[cfg(test)]` module.
2. Slice the exact production function or call-site block you want to guard.
3. Assert for the smallest set of textual invariants that carry the contract:
   - required enum variant at every call site
   - required filter (for example `WHERE state = 'active'`)
   - required retain/replace logic
   - required guard helper before a destructive action
4. Keep the assertion message explicit about the safety contract being protected.
5. Pair these tests with ordinary behavioral tests where possible; use source invariants to close the seam behavioral tests cannot cheaply or portably reach.

## Good fits in GigaBrain

- `finalize_pending_restore(...)` caller variants in `src/core/vault_sync.rs`
- watcher lifecycle filters/replacement logic in `src/core/vault_sync.rs`
- future hard-delete guard call-site proofs across `reconciler.rs` and `quarantine.rs`
- remap/restore ordering seams in `src/core/vault_sync.rs`, such as exact-ack-before-safety, safety-pipeline-before-`verify_remap_root(...)`, and "hold short-lived owner lease until inline `complete_attach(..., AttachReason::RemapPostReconcile)` finishes"
