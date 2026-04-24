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
