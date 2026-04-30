# Project Context

- **Owner:** {user name}
- **Project:** {project description}
- **Stack:** {languages, frameworks, tools}
- **Created:** {timestamp}

## Learnings

<!-- Append new learnings below. Each entry is something lasting about the project. -->
- 2026-04-30T08:30:31.626+08:00 — Nibbler rereviewed Batch 4 and found the `session_type` split is incomplete: online restore/remap handshake still accepts CLI leases as live owners because `session_is_live()` is untyped while handshake code trusts `collection_owners`.
- 2026-04-30T08:30:31.626+08:00 — Nibbler final-reviewed Batch 4 and confirmed the restore/remap handshake now relies on typed `live_collection_owner()` checks, making the partial checkpoint acceptable while task `12.7` remains honestly open.
- 2026-04-30T08:30:31.626+08:00 — Nibbler approved final Batch 4 closure: duplicate write-dedup insertion is now a typed fail-closed error, the duplicate cleanup path preserves the preexisting dedup entry, and task `12.7` is honestly closed without widening Batch 4 scope.
