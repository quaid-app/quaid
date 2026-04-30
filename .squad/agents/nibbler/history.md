# nibbler history

- [2026-04-29T07-04-07Z] History summarized and archived
- [2026-04-30T06:37:20.531+08:00] Rejected Batch 4 vault-sync review: `12.7` still overclaims concurrent dedup-failure proof because `insert_write_dedup()` discards duplicate inserts, and the new CLI live-owner routing still conflates short-lived offline leases with real serve ownership.
- [2026-04-30T08:30:31.626+08:00] Approved final Batch 4 closure: duplicate write-dedup insertion now fails closed with `DuplicateWriteDedupError`, the pre-rename duplicate path preserves the preexisting dedup entry while cleaning tempfile/sentinel, and task `12.7` is now honestly closed.
