---
name: "supersede-retrieval-coverage-honesty"
description: "How to test head-only supersede retrieval without lying to yourself"
domain: "quality"
confidence: "high"
source: "earned while auditing conversation-memory supersede/retrieval coverage"
---

## Use when

- A change adds head-only retrieval with an `include_superseded` escape hatch
- One integration test makes the slice look covered, but exact-slug or graph branches might still be unproved

## Required proofs

1. **Exact-slug retrieval proof**
   - Test the exact-slug helper or surface directly
   - Assert a superseded page is hidden by default and returned only when `include_superseded = true`

2. **Expansion-time filtering proof**
   - Seed progressive retrieval with a head page whose outbound link points at a superseded page
   - Assert default expansion skips the historical neighbour
   - Assert opt-in expansion restores it

3. **Graph proof**
   - Build a short `A -> B -> C` supersede chain
   - Assert graph traversal emits `superseded_by` edges as first-class edges, not typed-link stand-ins

## Why

Text-query integration alone can miss the real seams:
- exact-slug paths can short-circuit differently
- progressive expansion can leak archived pages back in
- graph traversal has its own supersede edge logic

If those three proofs are missing, coverage claims for the slice are probably overstated.
