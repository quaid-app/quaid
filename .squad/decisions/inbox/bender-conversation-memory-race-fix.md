# Bender decision: conversation-memory supersede race fix

- Timestamp: 2026-05-04T07:22:12.881+08:00
- Scope: `conversation-memory-foundations` tasks `2.2`-`2.5`
- Decision: `src/commands/put.rs` now stages the successor row and claims the predecessor head inside the same still-open SQLite write transaction before recovery-sentinel, tempfile, and rename work begins. The existing transactional `reconcile_supersede_chain` call stays in place after rename as the race backstop.
- Why: two different successor slugs could both preflight the same head and the loser surfaced `SupersedeConflictError` only after rename, which made the rejection contract dishonest because vault bytes could already be on disk.
- Trade-off: this keeps the SQLite writer transaction open across the Unix write-through seam. That wider single-writer window is accepted for this slice because it is the requested safe direction and it preserves the invariant that a rejected non-head supersede attempt does not mutate the vault.
