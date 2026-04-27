---
name: mcp-diagnostic-schema-review
description: Review frozen MCP diagnostic schemas by proving the real state predicate, precedence, and negative cases instead of only checking emitted labels.
---

# MCP Diagnostic Schema Review

Use this when a tool exposes state like `integrity_blocked`, `restore_in_progress`, or similar machine-readable diagnostics.

## Pattern

1. **Trace the full predicate**
   - Read the spec/design and list every storage/runtime condition behind each label.
   - Verify timestamp/age gates, in-progress vs terminal distinctions, and any runtime-only flags.

2. **Build a state matrix**
   - Cover positive arms, queued/in-progress variants, and negative lookalikes.
   - Include “label column set without terminal predicate” cases so the test proves fail-closed behavior.

3. **Prove precedence explicitly**
   - Create at least one case where multiple blockers coexist.
   - Assert the documented winner, not just that some non-null value appears.

4. **Guard slice boundaries**
   - If a broader tagged union or extra semantic is deferred, assert it does **not** surface yet.
   - Keep the field set frozen while narrowing values truthfully.

## Guardrails

- Don’t infer terminal state from a reason string alone when the contract requires a companion timestamp or age threshold.
- Don’t “helpfully” surface deferred semantics just because the backing column already contains them.
- For timeout-based states, test both sides of the threshold so reviewers can see the default/configured window is real.

## Docs schema-drift pattern (added 2026-04-25)

When documenting a new MCP tool response shape:

1. **Always verify against the struct**, not the design doc or PR description.
   - `grep -n "pub struct.*View" src/` — find the serialized view struct.
   - Read every `pub` field and its type. Cross-reference with the JSON example in docs.

2. **Check state enum arms**, not just the states listed in the design.
   - `grep -n "CollectionState\|as_str" src/core/vault_sync.rs` to find all enum arms.
   - Design may say `"needs_sync"` but code says `CollectionState::Detached`.

3. **Separate boolean flags from state values.**
   - `needs_full_sync`, `recovery_in_progress`, `integrity_blocked`, `restore_in_progress` are separate fields — not `state` enum arms.

4. **Note optional fields.** `root_path` is `Option<String>` populated only for `active` collections; docs should reflect `null` in other cases.

5. **Commit the JSON example as a contract.** Once shipped, MCP clients will codegen against it. A complete, accurate example beats a "representative" sketch with placeholder fields.
