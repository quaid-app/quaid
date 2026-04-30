# fry history

- [2026-04-29T07-04-07Z] History summarized and archived

## Learnings

- [2026-04-30T06:37:20.531+08:00] Closing the rename-before-commit seam truthfully required eliminating path-based parent creation from `quaid put`; a tiny fd-relative `walk_to_parent_create_dirs` helper plus a source-invariant test was enough to prove the actual production ordering without widening restore or IPC scope.
- [2026-04-30T06:37:20.531+08:00] The safe Batch 4 CLI lane is root-scoped: `quaid put` should refuse any live same-root serve owner and otherwise hold a short-lived offline lease for the whole direct write, rather than inventing a partial proxy mode.
- [2026-04-30T08:30:31.626+08:00] Closing 12.7 truthfully required defining duplicate write-dedup insertion as an explicit fail-closed invariant; on a Windows host, the honest proof mix is a production source-invariant test plus a Unix-only regression kept for Unix CI, rather than pretending the live vault-write path was executed locally.
