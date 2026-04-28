# Decision: PR #110 compile fix stays inside existing watcher fallback contract

**Date:** 2026-04-28  
**By:** Fry  
**PR:** #110 (`fix/no-direct-main-guardrails`)

## Decision

Fix the rerun-only compile break by preserving the current native-watcher fallback design:

1. Keep native watcher init failures as `Result<WatcherHandle, String>` inside the local `native_result` branch, so `recommended_watcher`, `configure`, and `watch` failures still fall back to poll mode.
2. Do **not** widen `VaultSyncError` with a new `String`/`notify::Error` conversion just to satisfy `?`.
3. Repair the watcher-health `CollectionInfoOutput` test fixture by explicitly setting the three runtime-only watcher fields to `None`.

## Why

- The branch logic already treats native watcher init failure as non-fatal and logs `watcher_native_init_failed ... falling_back_to_poll`; changing the error type at the outer function boundary would subtly change that contract.
- The missing `CollectionInfoOutput` fields are a fixture drift bug, not a product-behavior change.
- This keeps the fix minimal and avoids widening product surface area on a branch whose purpose is CI guardrails, not watcher redesign.
