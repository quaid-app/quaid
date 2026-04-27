## ADDED Requirements

### Requirement: Collection as first-class entity

The system SHALL store every page within a named collection. Each collection SHALL have a unique name, a root path on the local filesystem (`collections.root_path TEXT NOT NULL` — storage always retains the last-known absolute path even when the collection is `detached`, so that `sync --remap-root` can be attempted without first re-attaching and operators can see where the collection was last bound), an ignore-pattern list, and a boolean flag indicating whether it is the default target for agent-initiated writes. The MCP surface (`memory_collections`) presents `root_path` as `null` when `state != 'active'` because a detached/restoring root is not safe to walk or open, even though the underlying column is non-null — the API shapes the view, storage retains history.

#### Scenario: Fresh init creates default collection

- **WHEN** a user runs `quaid init` in a directory with no existing `memory.db`
- **THEN** the system creates `memory.db`, creates a `collections` row named `default` with `root_path = ~/.quaid/default-vault/` (creating the directory if missing), and sets `is_write_target = 1`

#### Scenario: Adding a new collection — atomic `.quaidignore` parse gates creation

- **WHEN** a user runs `quaid collection add work ~/Documents/work-vault` and any `.quaidignore` at the vault root parses cleanly (or is absent)
- **THEN** the system inserts a `collections` row with name `work`, absolute-resolved root path, `ignore_patterns` populated with the validated `.quaidignore` user patterns ONLY (per the file-authoritative contract — built-in defaults are NOT merged into the stored mirror; they are applied in code at reconciler-query time) or NULL if the file is absent, and `is_write_target = 0`
- **AND** the initial walk ingests all non-ignored `.md` files under the root

#### Scenario: `collection add` refuses when `.quaidignore` fails atomic parse

- **WHEN** a user runs `quaid collection add work ~/Documents/work-vault` and the vault's `.quaidignore` contains any invalid glob
- **THEN** `collection add` SHALL refuse — return a non-zero exit with the parse error details (line number + raw line + error message for each invalid line), and NOT create the `collections` row, NOT run the initial walk, NOT mutate any DB state
- **AND** the command message instructs the user to fix `.quaidignore` and re-run `add`
- **AND** there is NO last-known-good pattern set to fall back to (this is a brand-new collection) — the atomic-parse failure therefore prevents ANY ingest until the file is fixed. This ensures a privacy-sensitive `.quaidignore` is never silently bypassed on initial ingest.

#### Scenario: Name collision rejected

- **WHEN** a user runs `quaid collection add work ~/other-path` and a collection named `work` already exists
- **THEN** the command errors with a message referencing the existing collection; no database changes are made

#### Scenario: Setting write target is exclusive

- **WHEN** a user runs `quaid collection add memory ~/memory --write-target` and another collection is currently `is_write_target = 1`
- **THEN** the system clears the previous write target and sets it on the new collection in a single transaction; exactly one collection has `is_write_target = 1` at all times

### Requirement: Composite page identity

The `pages` table SHALL use `(collection_id, slug)` as its uniqueness key, with `id` as the primary key for foreign-key references. Slugs SHALL be unique within a collection but MAY repeat across collections. `memory_put` SHALL preserve the existing optimistic-concurrency contract: updates to a page that already exists at a given `(collection_id, slug)` REQUIRE an `expected_version` matching the current `pages.version`. A `memory_put` targeting an existing page without `expected_version` SHALL return `ConflictError` — never last-write-wins. Only the create path (no prior page at the slug) MAY proceed without `expected_version`.

#### Scenario: Same slug in two collections is allowed

- **WHEN** the `work` collection has a page with slug `notes/meeting` and the user creates a page with slug `notes/meeting` in the `personal` collection
- **THEN** both pages coexist; each has a distinct `pages.id`; neither is overwritten

#### Scenario: Create without `expected_version` is permitted when no prior page exists

- **WHEN** no page exists for `(work, notes/new)` and an agent calls `memory_put("work::notes/new", content)` without `expected_version`
- **THEN** a new page is created at version 1; no conflict occurs (there was nothing to be concurrent with)

#### Scenario: Update without `expected_version` is rejected with `ConflictError`

- **WHEN** a page exists for `(work, notes/meeting)` at version 5 and an agent calls `memory_put("work::notes/meeting", content)` WITHOUT supplying `expected_version`
- **THEN** `memory_put` returns `ConflictError` with a message indicating that updates must supply `expected_version` (plus the current version in the DB for the agent to fetch-and-retry)
- **AND** no tempfile is written; no DB mutation occurs; no filesystem mutation occurs
- **AND** this behavior is identical whether the caller is MCP (`memory_put` tool), CLI (`quaid put`), or any other future write entry point — the contract is per-write-operation, not per-interface, so stale writers cannot silently overwrite newer content across process boundaries

#### Scenario: Update with matching `expected_version` proceeds

- **WHEN** a page exists for `(work, notes/meeting)` at version 5 and an agent calls `memory_put("work::notes/meeting", content, expected_version=5)`
- **THEN** the version check passes, the filesystem precondition runs next, and on success the page's `version` bumps to 6

#### Scenario: Update with stale `expected_version` returns `ConflictError`

- **WHEN** a page exists for `(work, notes/meeting)` at version 6 (a concurrent writer advanced it) and an agent calls `memory_put("work::notes/meeting", content, expected_version=5)`
- **THEN** `memory_put` returns `ConflictError` on the version check before any tempfile write or DB mutation

### Requirement: External addressing by `<collection>::<slug>` string with ambiguity protection

MCP tools and CLI commands SHALL accept a single-string slug argument. The external form for explicitly addressing a page in a named collection is `<collection>::<slug>` using the literal two-colon separator `::`. Slugs themselves remain path-shaped with forward-slash separators (e.g., `people/alice`, `notes/meeting`). Using `::` as the external separator ensures that no existing slug ever collides with a collection prefix: `/` never appears as a collection-routing separator, so adding a collection named `people`, `notes`, `work`, or any other string cannot change the meaning of an existing path-shaped slug. Resolution rules:

1. If the argument contains `::`, the system SHALL split on the first `::` occurrence and treat the left side as a collection name and the right side as a slug. The collection name is looked up in `collections`; an unknown name returns a structured error. The slug is passed through verbatim (no further parsing) except for path-traversal rejection.
2. If the argument does NOT contain `::`, it is a "bare slug" and the system SHALL apply ambiguity-aware resolution. The `op_kind` classification MUST match the task 2.3 matrix exactly — a tool that mutates ANY DB state referencing the resolved page is a Write op regardless of whether it writes the `pages` row itself. Misclassifying a mutating tool as a Read is a trust-boundary bug: it would permit a bare slug to resolve to a unique non-write-target collection and mutate it (wrong-collection write), AND it would bypass the `CollectionRestoringError` interlock (task 11.8) that freezes a collection during restore. Round-16 reclassification aligns this matrix with the authoritative task 13.1 list.
 - **Single-collection memory** (only one collection exists): the bare slug resolves to that collection. No ambiguity possible. Write ops still go through the `restoring`-state interlock.
 - **Multi-collection memory** (two or more collections exist):
 - **Read ops** — non-mutating only (`memory_get`, `memory_backlinks`, `memory_timeline`, `memory_graph`, `memory_tags`, `memory_list`, `memory_link_close` in its lookup-only mode, **and `memory_gap` WITHOUT a slug** — memory-wide gap form that logs a `knowledge_gaps` row with `page_id = NULL`, resolves no collection, takes no interlock, and SUCCEEDS during `state = 'restoring'` on any/all collections per the /49 slug-less Read carve-out): count collections that contain a page with this slug. Zero → `NotFoundError`. Exactly one → resolve to that collection. Multiple → `AmbiguityError` listing the matching collections. For slug-less `memory_gap` there is no slug to resolve — the call bypasses the bare-slug resolution matrix entirely and logs memory-wide gap directly.
 - **Write ops — `WriteCreate`** (`memory_put` without `expected_version`): count collections that contain a page with this slug. Zero owners → resolve to write-target collection (create path). Exactly one AND that one is the write-target → resolve to it (treat as a CREATE is a version-check error unless the existing page matches — see `memory_put` contract). Exactly one but a DIFFERENT collection → `AmbiguityError` (refuse to silently shadow-create across collections). Multiple → `AmbiguityError`.
 - **Write ops — `WriteUpdate`** (every DB-mutating tool that references an existing page): `memory_put` with `expected_version`; **`memory_check`** (inserts `assertions`/`contradictions`); **`memory_raw`** (inserts `raw_data`); **`memory_link`** (inserts programmatic `links`, for BOTH source and target slug resolution — no "Read for lookup" carveout because the call result is a programmatic link referencing both pages); **`memory_link_close`** mutation mode (closes a programmatic `links` row); **`memory_gap` WITH a slug** (slug-bound form only — inserts `knowledge_gaps` with non-NULL `page_id` referencing the resolved page; the slug-less memory-wide form is classified as Read in the list above and has no bare-slug resolution path because there is no slug to resolve). Resolution: count collections owning the slug. Zero → `NotFoundError` (cannot mutate state for a page that doesn't exist). Exactly one → resolve to that collection regardless of write-target status (the page exists only there; mutating "the" page is unambiguous). Multiple → `AmbiguityError`. There is NO "silently route to write-target when write-target does not own the slug" branch — that would be the wrong-collection-mutation bug. For every Write op, the resolved collection's `state` AND `needs_full_sync` MUST be checked per task 11.8 BEFORE any filesystem or DB mutation — the gate is **OR-composed**: `CollectionRestoringError` SHALL be returned if `state = 'restoring'` OR `needs_full_sync = 1`. Either condition alone triggers the refusal; the second arm covers the post-Tx-B pre-attach window, the post-remap-DB-update pre-attach window, and the watcher-overflow reconcile window where `file_state` is known to be incomplete. `CollectionRestoringError` takes precedence over success.
 - **Write ops — `WriteAdmin`** (collection-level commands that mutate collection config, `.quaidignore`, or file frontmatter without a per-page slug): the canonical WriteAdmin set is `quaid collection ignore add`, `quaid collection ignore remove`, `quaid collection ignore clear --confirm`, `quaid collection migrate-uuids`, `quaid collection add --write-quaid-id` when applied to an existing collection, any future `--set-write-target`, and any future collection-level admin mutator. These operate on a collection identity rather than a slug, so they do not go through `parse_slug`; they resolve by collection name only. They MUST enforce the **OR-composed write-gate interlock** per task 11.8 — `CollectionRestoringError` SHALL be returned if `state = 'restoring'` OR `needs_full_sync = 1` — and the restore/remap handshake rules in the vault-sync spec — e.g., `quaid collection migrate-uuids work` issued while `work` is in `state = 'restoring'` OR `needs_full_sync = 1` SHALL return `CollectionRestoringError` before any frontmatter rewrite, and `quaid collection ignore add work "private/**"` issued in the same gate-satisfying state SHALL return `CollectionRestoringError` before any `.quaidignore` write. This prevents the confidentiality-regression case where an ignore change OR a UUID rewrite against the old root becomes effective against the new root after finalize. The spec-consistency audit (task 17.17) enforces that every authoritative WriteAdmin enumeration lists this full set.
 - **Search/list operations** that accept a slug filter: same as Read ops (exactly-one or error).

The `AmbiguityError` SHALL name every collection involved and include the full `<collection>::<slug>` forms so the caller can pick one. MCP tool responses SHALL surface this as a structured error, not a stale or misdirected result.

Collection names SHALL NOT contain the literal `::` sequence. `quaid collection add <name> <path>` SHALL reject any name containing `::` with a clear error. This keeps the parsing rule trivially unambiguous (first-`::`-wins on the external address) without reserved-word management against slug namespaces.

#### Scenario: Full form resolves correctly with `::` separator

- **WHEN** an agent calls `memory_get("work::notes/meeting")` and collection `work` exists with a page at slug `notes/meeting`
- **THEN** the system returns that page

#### Scenario: Path-shaped slug not mistaken for collection prefix

- **WHEN** memory has a collection named `work` and another collection named `people`, and an agent calls `memory_get("people/alice")`
- **THEN** the system does NOT interpret `people` as a collection name (the argument has no `::`); it treats the entire string `people/alice` as a bare slug and applies ambiguity-aware resolution
- **AND** if exactly one collection contains a page at slug `people/alice`, it resolves to that page
- **AND** if both `work` and `people` contain such a page, returns `AmbiguityError` instructing the caller to use `work::people/alice` or `people::people/alice`

#### Scenario: Collection name containing `::` rejected at creation

- **WHEN** a user runs `quaid collection add foo::bar ~/vault`
- **THEN** the command errors with a message that collection names cannot contain `::`; no `collections` row is inserted

#### Scenario: Bare form in single-collection memory

- **WHEN** only the `default` collection exists and an agent calls `memory_get("notes/meeting")`
- **THEN** the system resolves to `(default, notes/meeting)` and returns the page if found, or `NotFoundError` otherwise

#### Scenario: Bare form for read in multi-collection memory — unique match

- **WHEN** collections `work` and `personal` exist, a page `notes/meeting` exists only in `work`, and an agent calls `memory_get("notes/meeting")`
- **THEN** the system resolves to `(work, notes/meeting)` and returns that page
- **AND** logs the disambiguation at DEBUG

#### Scenario: Bare form for read in multi-collection memory — no matches

- **WHEN** collections `work` and `personal` exist, no page with slug `notes/nonexistent` exists in either, and an agent calls `memory_get("notes/nonexistent")`
- **THEN** the system returns `NotFoundError` (no ambiguity; no candidate pages exist)

#### Scenario: Bare form for read in multi-collection memory — ambiguous match

- **WHEN** collections `work` and `personal` exist, both contain a page `notes/meeting`, and an agent calls `memory_get("notes/meeting")`
- **THEN** the system returns `AmbiguityError` with a message like: "Slug `notes/meeting` exists in multiple collections: `work::notes/meeting`, `personal::notes/meeting`. Specify the full form."
- **AND** no page content is returned

#### Scenario: Bare form for create in multi-collection memory — write target is only owner

- **WHEN** collections `work` (write-target) and `personal` exist, no page `notes/new` exists in either, and an agent calls `memory_put("notes/new", content)`
- **THEN** the system resolves to `(work, notes/new)` and creates the page there

#### Scenario: Bare form for write in multi-collection memory — shadow create refused

- **WHEN** collections `work` (write-target) and `personal` exist, a page `notes/meeting` exists in `personal` but NOT in `work`, and an agent calls `memory_put("notes/meeting", content)` without `expected_version`
- **THEN** the system returns `AmbiguityError` refusing to create a shadow page, with a message instructing the caller to use `work::notes/meeting` for an explicit create in the write-target or `personal::notes/meeting` to update the existing page

#### Scenario: Bare form for write in multi-collection memory — ambiguous update

- **WHEN** collections `work` (write-target) and `personal` exist, both contain `notes/meeting`, and an agent calls `memory_put("notes/meeting", content)`
- **THEN** the system returns `AmbiguityError`; no write occurs

#### Scenario: Mutating side-effect tool (`memory_check`) with bare slug routes to the unique owner

- **WHEN** collections `work` (write-target) and `reference` exist, page `notes/claim` exists ONLY in `reference`, and an agent calls `memory_check("notes/claim")` (bare slug)
- **THEN** the system resolves to `(reference, notes/claim)` because exactly one collection owns the slug; the call proceeds to run `extract_assertions` + `check_assertions` and writes any resulting rows into `assertions`/`contradictions` for that page. Routing is by unique-ownership, NOT by write-target — `memory_check` is a `WriteUpdate` that targets an existing page, so it goes to whichever collection owns the slug. The write-target flag only matters for `WriteCreate` (bare-slug create with zero owners).
- **AND** if ZERO collections owned the slug, the call would have returned `NotFoundError` — `memory_check` cannot fabricate a page regardless of the write-target setting
- **AND** if BOTH collections owned the slug, the call would have returned `AmbiguityError` naming the `<collection>::<slug>` candidates
- **AND** if `reference.state = 'restoring'`, the call returns `CollectionRestoringError` BEFORE running any assertion extraction or DB insert, per task 11.8

#### Scenario: Mutating side-effect tool refused during `restoring` state

- **WHEN** collection `work` is in `state = 'restoring'` and an agent calls any of `memory_check("work::notes/x")`, `memory_raw("work::notes/x",...)`, `memory_link("work::a", "work::b",...)`, `memory_link_close("work::linkid")`, or `memory_gap("work::notes/x",...)`
- **THEN** every such call returns `CollectionRestoringError` BEFORE any DB mutation or filesystem work
- **AND** read-only calls (`memory_get`, `memory_search`, `memory_query`, `memory_list`, `memory_backlinks`, `memory_timeline`, `memory_graph`, `memory_tags`) against the same collection continue to succeed during the restore — the interlock is scoped to Write ops only

#### Scenario: WriteAdmin op (`collection ignore add`) refused during `restoring` state

- **WHEN** collection `work` is in `state = 'restoring'` (either mid-online-handshake or mid-pending-finalize) and a user runs `quaid collection ignore add work "private/**"`
- **THEN** the command returns `CollectionRestoringError` BEFORE any UPDATE on `collections.ignore_patterns` and BEFORE any write to `<work_root>/.quaidignore`
- **AND** no DB mutation occurs; no filesystem write occurs; the restore/remap flow is unaffected
- **AND** the message instructs the user to wait for restore/remap to finalize (or run `sync --finalize-pending`) and retry — this prevents the confidentiality-regression case where an ignore change landed against the OLD root becomes silently effective against the NEW root after finalize
- **AND** an analogous refusal applies to `quaid collection ignore remove`, `--set-write-target`, and any other collection-level admin mutator

#### Scenario: Slug-less `memory_gap` succeeds during `state = 'restoring'`

- **WHEN** collection `work` is in `state = 'restoring'` (either mid-handshake or pending-finalize) and an agent calls `memory_gap` WITHOUT a slug — logging a memory-wide gap such as `memory_gap(query="what's the historical context for X?", context="...", confidence=0.7)` with no `slug` / no `::` prefix
- **THEN** the call SUCCEEDS: a `knowledge_gaps` row is inserted with `page_id = NULL`, no collection is resolved, no `CollectionRestoringError` interlock applies, and the row is visible via `memory_gaps` / `quaid gaps list` during the restore window
- **AND** the same call SUCCEEDS against any collection state — memory-wide gap form is collection-independent and intentionally remains available during recovery because the restore window is exactly when agents most need to record audit observations
- **AND** the INVERSE form — `memory_gap` WITH a slug targeting a page in `work` (e.g., `memory_gap(slug="work::notes/x",...)`) — SHALL return `CollectionRestoringError` per the `WriteUpdate` interlock in task 11.8; this split is canonical across the collections spec, agent-writes spec, design, and all task enumerations

#### Scenario: Path-traversal rejected

- **WHEN** an agent calls `memory_put("work::../etc/passwd",...)`
- **THEN** the call errors before any filesystem or database operation; the filesystem is not touched
- **AND** the same rejection applies to bare slugs containing `..` components or absolute paths

### Requirement: Collection ignore patterns (`.quaidignore` authoritative; DB column is cached mirror)

**Round-17 contract.** `<root_path>/.quaidignore` on disk is the sole source of truth for user-authored ignore patterns. `collections.ignore_patterns` is a cached mirror populated from the file on every successful atomic parse and SHALL NOT be independently authoritative. Built-in defaults — at minimum `.obsidian/**`, `.git/**`, `node_modules/**`, `_templates/**`, `.trash/**` — are merged in code at reconciler-query time and are NOT stored in `collections.ignore_patterns` (so the cached mirror reflects exactly what the file contains, making parity testing trivial). The sync is one-way (file → DB mirror), transactional (atomic parse writes all-or-nothing), and mtime-free: timestamps are NOT used as a precedence arbiter because they are unreliable across restore/remap, clock skew, and editor rename cycles. Last-writer-wins keyed by mtime vs. `updated_at` was formally rejected in as a weak arbiter for a confidentiality boundary.

**Absent-file behavior is three-way (/27 fail-closed — authoritative single contract):** (a) **no prior mirror** — `.quaidignore` is absent AND `collections.ignore_patterns IS NULL` (fresh `collection add` or a never-configured vault): the cached mirror stays NULL and only built-in defaults apply; reconciler walks normally. (b) **prior mirror exists, opt-out UNSET (default fail-closed)** — `.quaidignore` is absent AND `collections.ignore_patterns IS NOT NULL` AND `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` is unset: the mirror is UNCHANGED, `collections.ignore_parse_errors` records the canonical `file_stably_absent_but_clear_not_confirmed` refusal, a WARN log `ignore_file_absent_refused collection=<N>` is emitted, NO reconciliation runs, and previously-excluded content stays excluded. The user MUST either save a fully-valid `.quaidignore`, save an empty one, or run `quaid collection ignore clear <name> --confirm` to change the mirror. (c) **prior mirror exists, opt-out SET** — either `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` OR the user ran `ignore clear --confirm`: the mirror is cleared to NULL, reconciliation runs, and previously-excluded files are re-ingested. The confidentiality rationale: a transient delete / editor crash / sync-glitch / cross-machine copy is indistinguishable from intent, and `.quaidignore` protects `private/**`-style patterns too sensitive to arbitrate by absence alone; case (b) is the default. There is no other absent-file rule elsewhere in this spec or design — earlier draft wording that said "absent → defaults only" unconditionally is superseded by this three-way contract. On `collection add` against a vault that lacks `.quaidignore`, NO file is created — the user opts in by saving one later (which takes case (a)'s fresh-attach path). Moving `memory.db` alongside the vault preserves behavior because the next serve start re-reads `.quaidignore` (via the atomic-parse path at watcher startup) and refreshes the mirror; moving `memory.db` WITHOUT `.quaidignore` takes case (b) and preserves last-known-good exclusions.

#### Scenario: Default patterns exclude Obsidian internals

- **WHEN** a collection is added pointing at an Obsidian vault with a `.obsidian/workspace.md` file
- **THEN** `.obsidian/workspace.md` is not indexed (excluded by the built-in default `.obsidian/**`, applied in code at query time regardless of `.quaidignore` content)

#### Scenario: `.quaidignore` loaded on walk — file is authoritative

- **WHEN** a vault root contains a `.quaidignore` file with the line `drafts/**` and a collection is added or synced
- **THEN** the atomic parse validates the file, updates `collections.ignore_patterns` as the cached mirror containing exactly `drafts/**` (the file's user patterns, without the built-in defaults merged in — defaults are applied at query time), and runs reconciliation
- **AND** files under `drafts/` are excluded from the index

#### Scenario: CLI adds a pattern (file-first, mirror follows)

- **WHEN** a user runs `quaid collection ignore add work "private/**"` and `quaid serve` is running on `work`
- **THEN** the CLI writes the new contents of `<work_root>/.quaidignore` (existing user patterns + `private/**`) to disk first
- **AND** the watcher observes the change, runs the atomic parse, and refreshes `collections.ignore_patterns` as the cached mirror of the new file contents
- **AND** the CLI does NOT write directly to `collections.ignore_patterns` — any such write would bypass the atomic-parse validation and create a divergence path between file and mirror
- **AND** if the resulting file would be invalid (malformed pattern injected), the CLI validates BEFORE writing the file and refuses with a non-zero exit code; NEITHER the file NOR the mirror is mutated
- **AND** if the file is successfully written but a concurrent editor save corrupts the file before the watcher's parse runs, the watcher's atomic-parse path handles the corruption (last-known-good mirror preserved, errors recorded) — the CLI returns after the file-write, so parse errors are surfaced via `memory_collections` MCP / `quaid collection info` rather than the CLI exit code

#### Scenario: First-time attach with no prior mirror — `.quaidignore` absent → defaults only

- **WHEN** `quaid collection add <name> <path>` is invoked for a brand-new collection and `<path>/.quaidignore` does NOT exist (no cached mirror has ever been populated)
- **THEN** `collections.ignore_patterns` is NULL (no user patterns cached); the reconciler applies only the built-in defaults. This is safe because there is no last-known-good set to preserve.
- **AND** a user saving a new `.quaidignore` with valid patterns later triggers the watcher's atomic parse, populates the mirror, and runs reconciliation. After that first population, the fail-closed rule applies on any subsequent stable absence.

#### Scenario: Moving `memory.db` + vault to a new machine preserves ignore behavior

- **WHEN** a user copies `memory.db` and the vault directory (including `.quaidignore`) to a new machine and runs `quaid serve`
- **THEN** the startup path reads `.quaidignore`, runs the atomic parse, and refreshes `collections.ignore_patterns` as the cached mirror — the DB's prior mirror value is replaced with the file's current contents
- **AND** **fail-closed on missing file**: if `.quaidignore` was not copied (only `memory.db` and the vault tree) AND `collections.ignore_patterns` is non-NULL in the copied DB (a prior mirror exists), startup SHALL NOT clear the mirror. It records `ignore_parse_errors = file_stably_absent_but_clear_not_confirmed`, logs `ignore_file_absent_refused collection=<N>` at WARN, and preserves the mirror's last-known-good patterns. Previously-excluded pages stay excluded until the user either (i) saves a new `.quaidignore`, (ii) runs `quaid collection ignore clear <name> --confirm`, or (iii) sets `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` for the opt-out behavior. The ONLY case where startup proceeds with defaults-only is when BOTH `.quaidignore` is absent AND `collections.ignore_patterns` IS NULL (no prior mirror to preserve).
- **AND** at NO point does the system consult `collections.updated_at` vs. `.quaidignore` mtime to decide precedence — the file wins unconditionally when present

### Requirement: Live `.quaidignore` reload with atomic parse, debounced stable-absence, and immediate reconciliation

The `.quaidignore` file SHALL be treated as a watched control file. Any write, rename, or delete affecting `<collection_root>/.quaidignore` observed by the collection's file watcher SHALL enter the reload debounce window (see "Transient absence" below). After the debounce window settles, the system runs an atomic parse: every non-empty, non-comment line is validated via `globset::Glob::new`. The reload has exactly two outcomes:

- **Fully-valid parse** — `collections.ignore_patterns` is refreshed as the cached mirror of the file's user patterns; `collections.ignore_parse_errors` is cleared (NULL); an immediate reconciliation runs using the new pattern set. Files now matching an ignore pattern are hard-deleted or quarantined per the DB-only predicate; files now un-ignored are ingested (with `quaid_id` persisted to frontmatter ONLY when the collection is opted into write-back. **Absent-file presentation is NOT part of this path under the default** — see "Stable absence is fail-closed by default" below. Absence parses as "zero user patterns" (and enters this fully-valid branch) ONLY when `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` is set OR no prior mirror exists to preserve.
- **Any parse failure** — `collections.ignore_patterns` is UNCHANGED (last-known-good preserved); `collections.ignore_parse_errors` records the failed lines; NO reconciliation runs; previously-applied exclusions remain in force until the file is fixed and a subsequent fully-valid parse succeeds.

**Transient absence — fail-closed by default.** Many editors and sync tools implement save as `unlink(old) → write(new)` or `rename(tmp, target)` with a brief window where the target path does not exist. Sync clients (Dropbox, iCloud, rsync), remote filesystems, editor/plugin crashes, or delayed-replacement writes can leave `.quaidignore` absent for arbitrary durations with no intent to clear exclusions. Because `.quaidignore` is a confidentiality boundary (`private/**`-style patterns), the spec SHALL fail closed: stable absence does NOT clear user patterns under default settings. The reload path SHALL guard against this as follows:

- Every `.quaidignore` filesystem event (IN_DELETE, IN_MOVED_FROM, IN_CREATE, IN_MOVED_TO, IN_MODIFY, IN_CLOSE_WRITE, FSEvents equivalent) enters a per-collection **reload debounce buffer** with a quiet-period of `QUAID_IGNORE_RELOAD_DEBOUNCE_MS` (default `2000` — 2 seconds, intentionally longer than the per-file debounce because control-file correctness matters more than latency). The watcher SHALL NOT reload on each raw event; it SHALL wait until no new event has arrived for the full quiet period, then evaluate the final resting state.
- If, after the debounce window, `.quaidignore` exists and is readable, run the atomic parse against the observed contents (fully-valid OR any-parse-failure per above).
- **Stable absence is fail-closed by default.** If, after the debounce window, `.quaidignore` is STILL absent, the default behavior SHALL preserve last-known-good patterns: `collections.ignore_patterns` stays at its current value, `collections.ignore_parse_errors` records `file_stably_absent_but_clear_not_confirmed`, and NO reconciliation runs. Previously-applied exclusions stay in force until the user either (a) saves a new `.quaidignore` with any valid contents to override, OR (b) explicitly runs `quaid collection ignore clear <name> --confirm` (a WriteAdmin command that writes an empty `.quaidignore` and records the user's explicit intent to clear user patterns). Log `ignore_file_absent_refused collection=<N>` at WARN with a message instructing the user how to confirm the clear. This default makes sync-tool / editor-crash / remote-filesystem corner cases fail toward confidentiality preservation rather than unintended re-indexing.
- If the debounce window sees a delete-then-create-within-quiet-period sequence (typical for atomic-save editors), only the final post-create state is evaluated — the transient absence is absorbed by the debounce, `collections.ignore_patterns` never transitions to "empty user patterns" during the gap, and previously-applied exclusions stay in force throughout the window.
- If the atomic parse fails after the debounce window, `collections.ignore_patterns` stays at last-known-good exactly as with any parse failure; errors are recorded.
- While the debounce window is open, `collections.ignore_patterns` is NEVER mutated and reconciliation is NEVER triggered by ignore-file events. This guarantees that a transient disappearance cannot drop `private/**` protections even for a single reconciliation pass.
- **Opt-in auto-clear on stable absence.** Users who want the prior behavior (stable absence past debounce automatically clears user patterns, treating file deletion as an intentional edit) SHALL set `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1`. This env var reverses the default: after the debounce window, a stably-absent file parses cleanly to "zero user patterns" and reconciliation proceeds. Intended only for users who understand the tradeoff and accept the sync-crash / remote-filesystem exposure risk. Default (env var unset) is the fail-closed behavior described above.
- **Explicit clear command.** `quaid collection ignore clear <name> --confirm` (task 9.8-b) is the canonical way to drop all user patterns: it writes an empty `.quaidignore` to disk, which takes the normal atomic-parse path (fully-valid parse of empty file → zero user patterns → mirror cleared → reconciliation runs), AND the command logs an explicit-clear audit entry so ambiguity between "user deleted by mistake" and "user intentionally cleared" is recorded.

The reload SHALL NOT be deferred to the next manual `quaid collection sync` — a user who adds `private/**` to `.quaidignore` and saves a clean file expects those notes to stop appearing in search/MCP within the debounce window plus the reconciliation pass.

**CLI write contract — file-only.** `quaid collection ignore add|remove` is file-first and SHALL NOT write `collections.ignore_patterns` directly. The sequence is: (1) validate the resulting pattern set via atomic-parse dry-run BEFORE any disk write — a malformed pattern returns non-zero exit with NO file or DB mutation; (2) write the new `.quaidignore` contents to disk via the same fd-relative pattern as `memory_put` (tempfile → fsync → rename → fsync parent); (3) return success. The watcher's atomic-parse path refreshes `collections.ignore_patterns` as part of its normal event handling — the CLI does NOT mutate the DB mirror directly. If serve is NOT running, the mirror refresh runs on the next `quaid serve` startup's atomic-parse-at-startup step. This contract REPLACES the earlier "mutates both the DB column and the on-disk file" wording — only `reload_patterns()` is permitted to write `collections.ignore_patterns`, enforced by code-audit test 17.5qq9.

The `.quaidignore` file itself is NEVER indexed as a page regardless of `.md` status.

#### Scenario: Editing `.quaidignore` with a fully-valid edit hides newly-ignored notes within seconds

- **WHEN** `quaid serve` is running with an active watcher on `work`, a page `private/salary.md` is currently indexed, and a user saves a fully-valid `.quaidignore` adding `private/**`
- **THEN** atomic parse succeeds; `collections.ignore_patterns` is updated; `ignore_parse_errors` is cleared; reconciliation runs
- **AND** `private/salary.md` is hard-deleted or quarantined per the DB-only predicate
- **AND** after reconciliation, `memory_search` and `memory_get("work::private/salary")` no longer return the page
- **AND** the user did NOT need to run `quaid collection sync`

#### Scenario: Removing a pattern via a fully-valid edit re-ingests previously-excluded files

- **WHEN** `.quaidignore` contained `archive/**`, the user deletes that line saving a fully-valid file, and the watcher observes the change
- **THEN** atomic parse succeeds; `collections.ignore_patterns` no longer contains `archive/**`
- **AND** reconciliation walks with the new set; files under `archive/` appear as `new` and are ingested into `pages` (`pages.uuid` populated); `quaid_id` is persisted to the file's frontmatter ONLY if the collection is opted into write-back (attached with `--write-quaid-id`), otherwise the file's on-disk bytes are unchanged
- **AND** after reconciliation they appear in `memory_search` / `memory_list`

#### Scenario: Any invalid line REJECTS the whole reload — last-known-good preserved

- **WHEN** `.quaidignore` contains a mix of valid and invalid lines (e.g., an editor intermediate save corrupted `secret/**` into `**]`)
- **THEN** atomic parse fails; `collections.ignore_patterns` is UNCHANGED; `ignore_parse_errors` records the failing line; NO reconciliation runs
- **AND** previously-applied exclusions (including the valid `private/**`) remain fully in force; files under `secret/` stay excluded even though the on-disk line is broken
- **AND** a WARN log `ignore_reload_rejected collection=<name> error_count=1 errors=[(line=..., msg=...)]` is emitted
- **AND** `memory_collections` MCP output and `quaid collection info` surface the parse error so the user knows which line to fix
- **AND** a subsequent fully-valid edit clears `ignore_parse_errors` and runs reconciliation normally

#### Scenario: CLI `ignore add|remove` with a malformed pattern refuses before persisting

- **WHEN** a user runs `quaid collection ignore add work "**]"` (a malformed glob)
- **THEN** the CLI validates the resulting pattern set via the same atomic parse, fails, returns a non-zero exit code naming the invalid pattern
- **AND** NEITHER `collections.ignore_patterns` NOR `<work_root>/.quaidignore` is mutated; serve observes no change

#### Scenario: CLI `ignore add|remove` with a valid pattern writes the FILE only; watcher refreshes the mirror

- **WHEN** a user runs `quaid collection ignore add work "archive/**"` (valid) while `quaid serve` is running on `work`
- **THEN** the CLI (i) dry-runs the atomic parse against the proposed resulting contents; (ii) writes only `<work_root>/.quaidignore` to disk via the same fd-relative tempfile → fsync → rename → fsync parent pattern as `memory_put`; (iii) returns success
- **AND** the CLI does NOT write `collections.ignore_patterns` directly — only the watcher's `reload_patterns()` path mutates the mirror, after its debounce window settles
- **AND** within the debounce + parse window, serve observes the file write, runs the atomic parse, refreshes the mirror, and runs immediate reconciliation
- **AND** outcome is identical to an editor save: pages under `archive/` are hard-deleted or quarantined per the DB-only predicate

#### Scenario: Atomic-save editor temporarily unlinks `.quaidignore` — transient absence does NOT clear protections

- **WHEN** `.quaidignore` contains `private/**` (applied), and the user edits it via an editor (Vim, VS Code, Obsidian) that implements save as `unlink(old) → write(new)` or `rename(tmp, target)`, creating a ~50–500ms window where `.quaidignore` does not exist at the path
- **THEN** the watcher emits IN_DELETE (or IN_MOVED_FROM) followed by IN_CREATE (or IN_MOVED_TO) within the debounce quiet-period (`QUAID_IGNORE_RELOAD_DEBOUNCE_MS` = 2000ms default)
- **AND** during the debounce window, `collections.ignore_patterns` is NOT mutated; NO reconciliation runs; `private/**` protections stay fully in force
- **AND** after the debounce window settles (no more events for 2s), the watcher reads the final `.quaidignore` contents and runs the atomic parse against them; if `private/**` is still present in the new file, the mirror refresh is a no-op (same patterns) and no reconciliation side-effects fire
- **AND** at NO point during the transient absence are previously-indexed `private/**` pages exposed via `memory_search` / `memory_get` / MCP — the confidentiality boundary is preserved across atomic-save flows

#### Scenario: Stable absence past debounce REFUSES to clear user patterns

- **WHEN** `.quaidignore` becomes absent (via `rm`, sync-tool delete, editor crash, remote-filesystem hiccup, etc.) and no replacement appears within the debounce quiet-period (2s); `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` is UNSET (default)
- **THEN** the watcher does NOT clear the mirror: `collections.ignore_patterns` stays at last-known-good; `collections.ignore_parse_errors` records `file_stably_absent_but_clear_not_confirmed`; NO reconciliation runs; previously-applied exclusions (including `private/**`-style patterns) stay fully in force
- **AND** log `ignore_file_absent_refused collection=<name>` is emitted at WARN with a message instructing the user how to confirm the clear
- **AND** the user resolves the situation either by (i) saving a new `.quaidignore` with any valid contents (the normal atomic-parse path refreshes the mirror), or (ii) running `quaid collection ignore clear <name> --confirm` (WriteAdmin command that writes an empty `.quaidignore` — takes the normal atomic-parse path with explicit audit-log record of the intent to clear)
- **AND** this is the inversion of the prior default: confidentiality preservation wins over eventual-consistency convenience because sync/crash/remote-fs corner cases are indistinguishable from intentional clears without the user's explicit action

#### Scenario: `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` restores the prior auto-clear behavior

- **WHEN** `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE=1` is set AND a user stably deletes `.quaidignore` (no replacement past the debounce window)
- **THEN** the watcher runs the atomic parse against "empty" contents (stable-absence → zero user patterns); the parse succeeds; the mirror is cleared; reconciliation runs
- **AND** files previously excluded by user patterns but not by defaults are re-ingested as `new`
- **AND** log `ignore_file_absent_auto_cleared collection=<name>` is emitted at INFO
- **AND** this env var is explicitly opt-in — users who set it accept the risk that sync-tool / editor-crash / remote-filesystem transients past 2s will clear protective patterns

#### Scenario: Explicit clear via `ignore clear --confirm` always succeeds regardless of default

- **WHEN** a user runs `quaid collection ignore clear work --confirm` (WriteAdmin command) and serve is running on `work`
- **THEN** the CLI writes an empty `<work_root>/.quaidignore` to disk and records an explicit-clear audit entry (`ignore_clear_confirmed collection=work user=<uid>` at INFO)
- **AND** the watcher observes the file write, runs the atomic parse (empty file → zero user patterns → fully-valid), refreshes the mirror, and runs reconciliation
- **AND** this path works identically whether `QUAID_IGNORE_AUTO_CLEAR_ON_ABSENCE` is set or unset — explicit intent always clears

#### Scenario: `.quaidignore` itself is never indexed

- **WHEN** the watcher processes events for `.quaidignore`
- **THEN** no `pages` row is created or updated for the file itself

### Requirement: Active `raw_imports` lifecycle (rotation on every content change; invariant: every v5 page has an active row)

The `raw_imports` table SHALL hold EXACTLY one active row per page (`is_active = 1`) under the v5 invariant. Every content-changing write SHALL rotate the active row so restore always materializes the CURRENT bytes, not the first-ingest snapshot. "Every content-changing write" exhaustively means: (a) initial ingest of a file, (b) reconciler/watcher re-ingest after an external edit, (c) `memory_put` from an MCP client OR `quaid put` from CLI — whether the operation is a CREATE (no prior page) or an UPDATE, (d) the UUID write-back self-write triggered by the `quaid_id` lifecycle.

Because `memory_put` CREATE is a content-changing write, a page authored entirely via `memory_put` SHALL have an active `raw_imports` row after the call succeeds — there is no "agent-authored pages have no raw_imports" carveout. The restore path has no normal-flow `render_page()` branch: if the invariant is violated (zero active rows for a page), restore aborts with `InvariantViolationError` and the operator-driven `--allow-rerender` override is required to proceed, per the "Corruption-recovery" scenario under the "Atomic staged restore with verification" requirement. Under v5 the invariant holds by construction at every write site (tasks 5.4d–5.4f, 12.4c), and unit/integration tests assert `COUNT(*) WHERE is_active=1 = 1` after every write.

For every such write, the rotation SHALL occur in the same SQLite transaction as the `pages` / `file_state` update:

1. `UPDATE raw_imports SET is_active = 0 WHERE page_id = ? AND is_active = 1`.
2. `INSERT INTO raw_imports (page_id, import_id, is_active, raw_bytes, file_path, created_at) VALUES (?, ?, 1, ?, ?, ?)` where `import_id` is a fresh UUID identifying this rotation event and `raw_bytes` is the current on-disk bytes (or for `memory_put`, the new tempfile bytes).

The existing partial index `idx_raw_imports_active ON raw_imports(page_id, is_active) WHERE is_active = 1` gives O(1) lookup for restore and enforces the "at most one active row" invariant via application-side rotation.

**Bounded retention for inactive rows.** Prior `is_active = 0` rows SHALL be kept subject to an explicit retention policy so normal editing (autosave, frequent `memory_put`, UUID write-back) does not cause `memory.db` to grow in proportion to edit count. The policy combines a per-page count cap with an age threshold, evaluated per page, per rotation, and on a periodic sweep:

- **Per-page cap:** `QUAID_RAW_IMPORTS_KEEP` (default `10`) — keep at most the `N` most recent inactive rows per `page_id`, ranked by `created_at DESC` (tie-break by `rowid DESC`). Rows beyond the cap are deleted.
- **Age threshold:** `QUAID_RAW_IMPORTS_TTL_DAYS` (default `90`) — delete inactive rows whose `created_at < now() - <TTL>`, regardless of the cap.
- **Combined effect:** at steady state, every page has (1 active row) + (at most `N` inactive rows, each younger than TTL). A page edited 1,000 times in one day keeps ~10 snapshots, not 1,000. A page edited twice ever keeps 1 active + 1 inactive.
- **Active row is never collected.** The GC predicate is always `WHERE is_active = 0 AND...`. The v5 "exactly one active row per page" invariant is orthogonal to retention and is asserted independently.
- **Opt-out (audit mode):** `QUAID_RAW_IMPORTS_KEEP_ALL=1` disables the cap AND the TTL, restoring the original "retain everything" behavior. Intended for users running forensic or research workflows; not the default because it re-introduces unbounded growth.
- **When GC runs:** (a) **opportunistically, inline with every rotation** — immediately after inserting the new `is_active = 1` row in the write-sequence SQLite tx, the rotation SHALL also `DELETE FROM raw_imports WHERE page_id = ? AND is_active = 0 AND (rowid NOT IN (SELECT rowid FROM raw_imports WHERE page_id = ? AND is_active = 0 ORDER BY created_at DESC, rowid DESC LIMIT <N>) OR created_at < now() - <TTL>)`. All in the same tx as the rotation so retention is invariant-maintained on every write. (b) **Periodic daily sweep** — a background task in `quaid serve` (and a one-shot path via `quaid collection audit --raw-imports-gc`) scans ALL pages for rows that exceed the policy and deletes them. Needed because `QUAID_RAW_IMPORTS_TTL_DAYS` can expire rows without any write event to trigger inline GC (a page that is never touched after an edit still ages past the TTL).
- **Restore is unaffected.** Restore always reads `is_active = 1` only. Retention changes have zero effect on the restore path.
- **`quarantine export` is best-effort against retained history.** The JSON export MAY include all inactive rows that survive retention at export time. The export documents which rows were captured and, if the policy elided older rows, the export header records that (count omitted = N, oldest retained = T). Users who need full history set `QUAID_RAW_IMPORTS_KEEP_ALL=1` BEFORE the rotation they care about — there is no post-hoc recovery of a deleted inactive row.
- **Operator visibility.** `quaid collection info` reports per-collection `raw_imports_inactive_count` and the effective `keep` / `ttl_days` values. `quaid stats` aggregates across collections.

#### Scenario: Edit after first ingest rotates raw_imports; restore reflects latest bytes

- **WHEN** a file `notes/meeting.md` is ingested initially (raw_imports row #1 active), then edited externally so the watcher re-ingests it (raw_imports row #2 with new bytes active, row #1 marked inactive in the same tx), and the user runs `quaid collection restore`
- **THEN** restore reads `raw_imports` filtered by `page_id = <meeting.id> AND is_active = 1`, finds exactly one row (#2), and writes its `raw_bytes` to disk
- **AND** the restored file's sha256 equals sha256 of the LATEST edited bytes, not the original first-ingest bytes
- **AND** row #1 remains in the table with `is_active = 0` (available for audit / historical export) but is NOT used by restore

#### Scenario: `memory_put` rotates raw_imports so restore reflects the agent write

- **WHEN** an MCP client calls `memory_put("work::notes/n", new_content)` updating a page
- **THEN** in the SINGLE SQLite transaction of the rename-before-commit write sequence (alongside pages / file_state / embedding_jobs updates), the prior active raw_imports row for the page is marked `is_active = 0` and a new row is inserted with the new content bytes (now durably on disk post-rename) and `is_active = 1`
- **AND** a subsequent `quaid collection restore` writes those `memory_put` bytes (not the pre-memory_put bytes); because the rotation is in the same tx as the rest of the commit, a crash between rename and commit leaves neither the old raw_imports flipped to inactive nor a new row inserted — the reconciler re-ingests from disk, which produces a fresh rotation matching the actual on-disk bytes

#### Scenario: UUID write-back rotates raw_imports so restored files contain `quaid_id`

- **WHEN** a file is ingested without `quaid_id` in frontmatter; the system generates a UUIDv7 (raw_imports row #1 active with UUID-less bytes), then self-writes the UUID into frontmatter (raw_imports row #2 active with UUID-bearing bytes, #1 inactive); the user later runs `quaid collection restore`
- **THEN** the restored file contains `quaid_id: <X>` in its frontmatter (from row #2), closing the concern that a restore from a pre-UUID snapshot would produce a file that later rename detection cannot track by UUID

#### Scenario: Exactly one active row per page (invariant — no carveouts)

- **WHEN** the rotation is executed in the same tx as the page update (UPDATE-then-INSERT order)
- **THEN** for EVERY page in the collection — including pages authored entirely via `memory_put`, pages that have only ever been updated programmatically, and pages created during the UUID write-back path — `SELECT COUNT(*) FROM raw_imports WHERE page_id = ? AND is_active = 1` equals exactly 1
- **AND** no category of page is permitted to have zero active rows under v5; any observation of zero is definitionally an invariant violation handled by the corruption-recovery path described under "Atomic staged restore with verification"
- **AND** the condition is asserted by unit tests after every write site (ingest, reconciler re-ingest, watcher update, `memory_put` CREATE, `memory_put` UPDATE, UUID self-write) and by the `quaid collection audit` command

#### Scenario: Inactive rows remain available to `quarantine export` (subject to retention)

- **WHEN** a page is quarantined and the user runs `quaid collection quarantine export` on it
- **THEN** the JSON export MAY include the active `raw_imports` row and any inactive rows that SURVIVE the retention policy at export time (per-page `KEEP` cap and `TTL_DAYS` age threshold), so the export is a self-contained record of the retained content history
- **AND** the export header records the effective `keep` and `ttl_days` values in force at export time, so a consumer can determine whether older bytes were collected rather than missing
- **AND** users who need byte-for-byte history of every edit set `QUAID_RAW_IMPORTS_KEEP_ALL=1` BEFORE the edits they want to preserve; the retention policy does NOT offer post-hoc recovery of already-GC'd rows

#### Scenario: Inline GC enforces per-page cap during rotation

- **WHEN** a page has `QUAID_RAW_IMPORTS_KEEP = 10` and already carries 1 active + 10 inactive rows, and a new content-changing write occurs
- **THEN** in the same SQLite tx: the active row is flipped to `is_active = 0`, the new row is inserted with `is_active = 1`, AND a GC DELETE removes the oldest inactive row (the one that is now the 11th youngest by `created_at DESC`, beyond the `KEEP` cap)
- **AND** post-rotation invariant: the page has exactly 1 active + at most 10 inactive rows; the GC is atomic with the rotation (no window where the page has 11 inactive rows)

#### Scenario: Inline GC enforces TTL during rotation

- **WHEN** a page has 1 active + 3 inactive rows where the oldest inactive row has `created_at = now() - 120 days` and `QUAID_RAW_IMPORTS_TTL_DAYS = 90`
- **THEN** on the next rotation, the same tx that inserts the new active row also DELETEs the 120-day-old inactive row (age > TTL) even though the `KEEP` cap is not yet reached
- **AND** post-rotation, the page has 1 active + (at most) 3 inactive rows, all younger than TTL

#### Scenario: Periodic sweep GCs idle pages that age past TTL

- **WHEN** a page was edited once, producing 1 active + 2 inactive rows, and is then never touched again for 100 days, with `TTL_DAYS = 90`
- **THEN** the daily background sweep (or `quaid collection audit --raw-imports-gc`) DELETEs the 2 inactive rows because no rotation event would have triggered inline GC on this page
- **AND** the active row is untouched; restore continues to produce byte-exact recovery
- **AND** a summary log records `raw_imports_sweep deleted=<N> across pages=<M>` for operator visibility

#### Scenario: `KEEP_ALL=1` disables retention

- **WHEN** `QUAID_RAW_IMPORTS_KEEP_ALL=1` is set at serve start, a page is edited 1,000 times over 180 days
- **THEN** inline GC is skipped, the daily sweep is skipped for raw_imports, and the page accumulates 1 active + 1,000 inactive rows
- **AND** `quaid collection info` reports the effective policy as `keep=∞ ttl=∞ (KEEP_ALL=1)` so the operator sees that retention is disabled
- **AND** disabling `KEEP_ALL` later does NOT retroactively GC existing rows; only new rotations trigger inline GC against the now-active policy; the daily sweep backfills TTL-expired rows

### Requirement: Collection lifecycle commands

The system SHALL provide CLI subcommands to manage collections: `add`, `list`, `info`, `sync`, `remove`, `restore`, and `ignore {add,remove,list}`.

#### Scenario: `list` shows all collections with status

- **WHEN** a user runs `quaid collection list`
- **THEN** a table is printed with columns: name, root_path, write_target, page_count, detached (yes/no)

#### Scenario: `info` shows full detail

- **WHEN** a user runs `quaid collection info work`
- **THEN** the output includes: name, root_path, ignore_patterns, watcher_mode, watcher_status, last_sync_at, page_count, embedding_queue_depth, detached flag, writable (boolean), state (`active`/`detached`/`restoring`), AND recovery diagnostics when relevant: `pending_root_path` (string or null), `pending_command_heartbeat_age` (seconds since last refresh, or `"stale"` if NULL, or omitted if `pending_command_heartbeat_at IS NULL` — the operator reads this to verify the /69 reset precondition that the originator heartbeat has aged past `2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS`, default 60s), `integrity_failed_at` (timestamp or null — terminal blocking flag for manifest tampering), `pending_manifest_incomplete_at` (timestamp or null with computed `age_seconds` and `escalation_remaining_seconds` when present — so the operator sees whether recovery is still within the self-heal window or approaching escalation), `restore_command_id` (UUID or null — helps identify an in-flight restore), **`reconcile_halted_at` (timestamp or null)** and **`reconcile_halt_reason` (`"duplicate_uuid"` | `"unresolvable_trivial_content"` | null)**. When a collection is in a blocking state, the diagnostics block is prominent and includes a suggested operator command matched to the specific blocking signal: `"Run: quaid collection restore-reset work --confirm"` for terminal `integrity_failed_at` / escalated `pending_manifest_incomplete_at` states, `"Run: quaid collection sync work --finalize-pending"` for recoverable pending-finalize, and **`"Run: quaid collection reconcile-reset work --confirm"` (new command — task 9.7f) for terminal `reconcile_halted_at` states, with a pre-amble message instructing the operator to manually resolve the underlying vault condition first: for `duplicate_uuid` strip the duplicate `quaid_id` from every file except the one intended to retain identity; for `unresolvable_trivial_content` run `quaid collection migrate-uuids` first to write UUIDs to the trivial-content files or use `restore` to re-materialize the vault with known identities**. This makes the /69/83 operator recovery sequences executable without falling back to raw SQL. **MCP observability (/83; tagged discriminator + restore advisory).** The `memory_collections` MCP tool response adds TWO diagnostic fields to its frozen schema to let agents (i) identify the specific terminal-blocking cause AND pick the right reset command, AND (ii) warn users before editing during the narrow mid-restore window:

- `integrity_blocked: null | "manifest_tampering" | "manifest_incomplete_escalated" | "duplicate_uuid" | "unresolvable_trivial_content"` (— replaces the /83 boolean). `null` means the collection is NOT terminal-blocked; a string value names the specific blocking cause and maps directly to the operator command: `"manifest_tampering"` ← `integrity_failed_at IS NOT NULL` → `quaid collection restore-reset`; `"manifest_incomplete_escalated"` ← `pending_manifest_incomplete_at` aged past `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` → `quaid collection restore-reset`; `"duplicate_uuid"` ← `reconcile_halted_at IS NOT NULL AND reconcile_halt_reason = 'duplicate_uuid'` → operator strips duplicate `quaid_id` frontmatter manually, then `quaid collection reconcile-reset`; `"unresolvable_trivial_content"` ← `reconcile_halted_at IS NOT NULL AND reconcile_halt_reason = 'unresolvable_trivial_content'` → operator runs `migrate-uuids` offline (or `restore`) then `reconcile-reset`. Precedence when multiple causes co-exist: `manifest_tampering > manifest_incomplete_escalated > duplicate_uuid > unresolvable_trivial_content` (left-to-right: most-severe identity-corruption signal wins). Agents observing a non-null `integrity_blocked` SHALL (i) stop retrying, (ii) surface the string value to the user, AND (iii) map it to the correct reset command without shelling out to `quaid collection info`. Backwards-compat: truthiness checks (`if (integrity_blocked)...`) still branch correctly; strict schemas must widen `boolean` to `null | string`.
- `restore_in_progress: boolean` (— CLAUDE-F1 advisory). `true` when the collection is mid-restore in the destructive window (`state='restoring'` AND Phase 2 stability passed AND Tx-B has not cleared the pending columns); `false` otherwise. Agents SHALL treat `restore_in_progress=true` as a "do not edit this vault" advisory to surface to the user: the microsecond residual race between Phase 3's `snap_fence` and the destructive SQLite commit is documented as silently lossy for concurrent external writes (see `vault-sync/spec.md` honest disclosure). This is distinct from `state='restoring'` alone — a `restoring` state before Phase 2 has not yet consumed user bytes and is safe to edit; the advisory fires specifically once the capture is committing.

This extends the schema from 11 fields to 13 fields; audit invariant 17.17(f) is updated accordingly. The richer operator-facing diagnostics (`pending_root_path`, `pending_command_heartbeat_age`, `integrity_failed_at`, `pending_manifest_incomplete_at`, `restore_command_id`, `reconcile_halted_at`, `reconcile_halt_reason`) remain CLI-only via `quaid collection info` — agents now have sufficient per-cause information in `integrity_blocked` to choose between `restore-reset` and `reconcile-reset` without the richer set.

#### Scenario: `remove` without `--purge` refuses to drop collection with pages

- **WHEN** a user runs `quaid collection remove work` and the `work` collection has at least one page
- **THEN** the command errors with a message instructing the user to pass `--purge` if deletion is intended; no changes are made

#### Scenario: `remove --purge` cascades (offline OR with `--online` handshake)

- **WHEN** a user runs `quaid collection remove work --purge` and no live owner exists for the collection (either `collection_owners` has no row for it, OR the owning `serve_sessions.heartbeat_at` has aged past the 15s liveness threshold and has been swept)
- **THEN** all pages, `file_state` rows, programmatic/wiki links, assertions (import and non-import), `raw_data`, `contradictions`, embeddings, `embedding_jobs`, `raw_imports`, `tags`, and `timeline_entries` for that collection are deleted via `ON DELETE CASCADE` from `pages`; the `collections` row is deleted; the operation completes in a single SQLite tx
- **AND** liveness is resolved EXCLUSIVELY via `collection_owners` for the target collection — an unrelated serve session that happens to own a different collection does NOT make `work` appear online; only a row in `collection_owners` tied to this collection with a live `serve_sessions.heartbeat_at` does

#### Scenario: `remove --purge` with live owner — refused by default

- **WHEN** a user runs `quaid collection remove work --purge` while `collection_owners` has a row for `work` pointing to a live `serve_sessions` row (heartbeat within 15s)
- **THEN** the command errors with `ServeOwnsCollectionError` naming the owning pid/host joined from `serve_sessions`, and instructing the user to stop serve OR pass `--online`
- **AND** no mutation occurs

#### Scenario: `remove --purge --online` coordinates with the owning serve and completes

- **WHEN** a user runs `quaid collection remove work --purge --online` with the collection live-owned by session `S` (as recorded in `collection_owners.session_id`)
- **THEN** the command captures `expected_session_id = S` from `collection_owners` (NOT from any arbitrary `serve_sessions` row), verifies liveness via that session's `serve_sessions.heartbeat_at`, then in one tx sets `collections.state = 'restoring'` (used as a do-not-touch marker), computes `cmd_reload_generation = reload_generation + 1` and writes it back, NULLs the ack triple (`watcher_released_session_id`, `watcher_released_generation`, `watcher_released_at`), and sets `pending_command_heartbeat_at = now()`
- **AND** serve's per-collection supervisor observes the state change, stops the watcher, closes `root_fd`, releases resources, writes the ack triple in one tx (`watcher_released_session_id = S`, `watcher_released_generation = cmd_reload_generation`, `watcher_released_at = now()`), AND EXITS (removes its entry from the process-global `supervisor_handles` registry per task 11.7's contract)
- **AND** the command's lease-based handshake helper matches on all three fields within the timeout, proceeds with the purge cascade as above (all pages + all DB-only state including `knowledge_gaps` and `contradictions` cascade via `ON DELETE CASCADE` per v5 schema), then deletes the `collections` row
- **AND** **/54/55/56/57 abort semantics:** if the helper returns `ServeHandshakeTimeoutError` or `ServeDiedDuringHandshakeError`, the command SHALL run the same abort-path resume sequence as restore/remap in a FRESH SQLite connection — revert `state` to prior, keep `root_path` unchanged (no purge ran), NULL the ack triple, AND bump `reload_generation` to `cmd_reload_generation + 1`. The generation bump is an ordering marker for RCRT's sweep (task 9.7d), NOT a poll trigger for the (now-exited) supervisor. RCRT observes the owned collection with `state='active'` and no live `supervisor_handles` entry on its next sweep, acquires the per-collection single-flight mutex, opens a fresh `root_fd` against the original `root_path`, runs `full_hash_reconcile`, starts a new watcher, and spawns a new supervisor — all without requiring operator intervention or a serve restart. Expected reattach latency: up to `QUAID_DEFERRED_RETRY_SECS` (default 30s). Log `purge_abort_resumed collection=<N> prior_generation=<cmd_reload_generation> resume_generation=<cmd_reload_generation+1>` at WARN; exit non-zero; no data mutation
- **AND** on success, the collection row has been deleted; RCRT's next sweep finds no row matching the collection_id and takes no action; no orphan state remains; other collections remain active under their existing supervisors

#### Scenario: `remove` refuses to drop the write-target collection

- **WHEN** a user runs `quaid collection remove <write_target_name> --purge` and that collection currently has `is_write_target = 1`
- **THEN** the command errors, instructing the user to set another collection as write target first

### Requirement: State authority classification

The system SHALL distinguish two categories of persisted state:

1. **Vault-authoritative state** — state that can be fully regenerated by re-ingesting page markdown from the vault: page content (`compiled_truth`, `timeline`, `frontmatter`, `wing`, `room`, derived `title`, `type`, `summary`), `page_fts` (trigger-derived), `page_embeddings` and vec tables (regenerated by the embedding worker), wiki-link rows in `links` (extracted from markdown body via `extract_links()`), heuristic rows in `assertions` (produced by `check_assertions()` at ingest time), `timeline_entries` (derived from timeline markdown), and `tags` that originate from page frontmatter.

2. **DB-authoritative state** — state that CANNOT be reconstructed from page markdown alone and is therefore preserved only in `memory.db`: programmatic links created via the `memory_link` MCP tool (non-wiki-link typed relationships with `valid_from`/`valid_until` temporal scoping), programmatic assertions created via the `memory_check` MCP tool (contradiction detection with supersession chains), `raw_data` (external API sidecar content), `contradictions`, `knowledge_gaps` (user query history), `config`, `quaid_config`, `embedding_models` registry, `raw_imports` (original file bytes for byte-exact round-trip), and `import_manifest` (ingest audit trail).

The system SHALL surface this distinction to users through documentation and `quaid collection info` output so that users can make informed backup and recovery decisions.

#### Scenario: Authority categories are documented

- **WHEN** a user reads the collections/portability documentation in `docs/collections.md`
- **THEN** the two authority categories are listed with concrete table names for each, along with backup recommendations that pair `memory.db` with the vault directory for complete recovery

### Requirement: Portability and restore for vault-authoritative state

Moving `memory.db` to another machine SHALL preserve all DB-authoritative state and all vault-authoritative state that was indexed at the time of the move. The `quaid collection restore` command SHALL materialize vault-authoritative content for a collection back to disk at a specified target path so that the vault can be reconstructed from the DB alone. Restoring and then re-ingesting from the restored vault SHALL preserve vault-authoritative state exactly; DB-authoritative state SHALL be preserved because it travels inside `memory.db` itself.

#### Scenario: Detached collection is surfaced

- **WHEN** `memory.db` is moved to a new machine and opened, and a collection's `root_path` does not exist on the new machine
- **THEN** `quaid collection info <name>` shows `detached: yes` and watcher status is `stopped`; other collections continue to function

### Requirement: Atomic staged restore with verification (absent-target-only; two-phase with recoverable finalize)

`quaid collection restore` SHALL operate only against a target path that does not yet exist on the filesystem (or that exists as an empty directory). Under this precondition, the operation is atomic at the granularity of "collection now points at a verified restored vault" — never at "collection now points at a partially-written directory." The command SHALL refuse to run against a non-empty target, an existing file, or any target that would require destructive replacement. No `--force` flag is provided, because `rename()` cannot atomically replace a non-empty directory or a file on POSIX filesystems; users who need to restore onto an occupied path must remove or rename the existing content themselves first, making the destructive step explicit and out-of-band from the Quaid tool.

**Two-phase structure to eliminate post-rename orphaned state.** The earlier single-phase description ("rename, then update root_path in one tx") had a failure mode: if the final SQLite tx failed after the rename, the restored vault lived on disk at the target path but `collections.root_path` still pointed at the old location, and retry was blocked because the target was no longer absent. The flow is now two-phase with a `pending_root_path` intent column that makes post-rename failure recoverable:

- **Schema addition.** `collections.pending_root_path TEXT NULL DEFAULT NULL` plus `collections.pending_restore_manifest TEXT NULL DEFAULT NULL` (— per-file sha256 manifest + rename-time `(inode, device_id)` tuple) plus `collections.integrity_failed_at TEXT NULL DEFAULT NULL` (— blocking flag set when manifest re-validation fails). Set pre-rename to the intended target, cleared post-finalize. Their presence together with `state = 'restoring'` is the recoverable "rename happened, finalize did not" signal.
- **Pre-rename phase (stage + verify + intent + manifest).** The command stages the restored tree in a sibling directory, verifies file count and per-file sha256 against `raw_imports`-derived expectations, then in a **Tx-A** writes its intent: `state = 'restoring'` (already set if `--online` handshake opened one; a no-op update if so) AND `pending_root_path = <target_path>` AND `pending_restore_manifest = <JSON with per-file sha256 + file count + rel paths>`. For offline restore, Tx-A also bumps `reload_generation`. Tx-A happens BEFORE the filesystem rename.
- **Rename.** Atomically `rename()` the staging directory onto the (absent or empty) target. Immediately after rename, `fstat` the target root and record `rename_inode_dev = (inode, device_id)` into `pending_restore_manifest` via a brief fresh-connection tx. **Post-rename manifest update — bounded retry before escalation.** SQLite's common transient failures (WAL busy-timeout, brief I/O hiccup) are NOT evidence of tampering. The command SHALL retry the `rename_inode_dev` write with bounded exponential backoff: 3 attempts with 100ms → 500ms → 2s delays, each on a fresh SQLite connection. If an attempt succeeds, the restore proceeds normally to Tx-B. If all three attempts fail, escalate: the command enters a degraded `PendingManifestIncomplete` state that is RECOVERABLE, not immediately operator-reset-blocking. In a best-effort fresh-connection tx the command sets a new `pending_manifest_incomplete_at = now()` field (schema addition — task 1.1 update), leaves `pending_restore_manifest` in its pre-rename shape (per-file sha256 still present, `rename_inode_dev` absent), and logs `restore_manifest_update_failed collection=<N> pending_root_path=<P> attempts=3` at WARN. Recovery semantics: `finalize_pending_restore()` encountering `pending_manifest_incomplete_at IS NOT NULL` SHALL retry the `rename_inode_dev` write ONCE inline (fresh connection, same backoff); on success it proceeds to manifest verification with the just-recorded tuple; on further failure it defers via the normal `Deferred` outcome so RCRT's next sweep (task 9.7d — the sole runtime recovery actor per the /57 single-actor contract) picks it up every `QUAID_DEFERRED_RETRY_SECS` (default 30s). Only after `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` (default 1800s / 30 min) of persistent failure does `finalize_pending_restore()` escalate to `IntegrityFailed` and set `integrity_failed_at` — at which point manual `restore-reset` is legitimately required because the DB is unable to accept the identity-tuple write for a sustained period. This makes transient SQLite failures self-healing and reserves the operator-reset block for genuine durable failures. If even the best-effort `pending_manifest_incomplete_at` write fails, treat a manifest missing `rename_inode_dev` the same way on the next recovery pass — no state is silently adopted.
- **Finalize phase (Tx-B).** A single SQLite transaction runs ONLY AFTER manifest re-validation passes (see recovery contract below). Tx-B sets `root_path = <target>`, NULLs `pending_root_path`, NULLs `pending_restore_manifest`, NULLs `integrity_failed_at`, NULLs `pending_manifest_incomplete_at`, NULLs `pending_command_heartbeat_at`, NULLs `restore_command_id`, `state = 'active'`, **SETS `needs_full_sync = 1`** (write-gate — arms the write-interlock so `memory_put` / all WriteCreate/WriteUpdate/WriteAdmin are refused with `CollectionRestoringError` until RCRT's attach-completion clears the flag; closes the post-Tx-B pre-attach hole where `memory_put`'s canonical external-create precondition would misclassify every restored page as an unindexed external create because Tx-B also `DELETE FROM file_state`), bumps `reload_generation` again, clears the ack triple, and `DELETE FROM file_state WHERE collection_id = ?`. `file_state` for the new root is NOT inserted here — RCRT's next sweep (task 9.7d) observes the owned collection with `state = 'active' AND needs_full_sync = 1` and no live `supervisor_handles` entry, acquires the per-collection single-flight mutex, opens a fresh `root_fd` against the new `root_path`, runs `full_hash_reconcile` (which re-populates `file_state`), commits the attach-completion tx clearing `needs_full_sync = 0` (which opens the write-gate), THEN starts a new watcher, and spawns a new per-collection supervisor. Under the /55/56/57/58/59 single-actor contract, RCRT is the sole runtime attach actor — the per-collection supervisor (task 11.7) exited at release and does NOT poll for `restoring → active` transitions.
- **Tx-B failure — recoverable, not stranded (/59 fix — NOT routed through needs_full_sync).** If Tx-B fails (WAL lock past busy-timeout, SQLite reports any error), the command SHALL: (a) NOT set `collections.needs_full_sync = 1` — the generic recovery worker (task 6.7a) reconciles against `collections.root_path`, which at this point still points at the OLD vault while `pending_root_path` holds the new target; setting the flag would reconcile the wrong tree and clear it without adopting the restored vault. The generic worker is now state-gated to `state = 'active'` only, so even if `needs_full_sync` leaked it would be skipped — but explicitly not setting it removes the ambiguity. (b) Log `restore_finalize_pending collection=<N> pending_root_path=<P>` at ERROR. (c) Return non-zero telling the user that recovery will proceed via `finalize_pending_restore` — either the command itself retries under `FinalizeCaller::RestoreOriginator { command_id }` while still alive (task 9.7 (l)), OR after the command dies RCRT's sweep picks it up as `FinalizeCaller::StartupRecovery { session_id }` on the next `quaid serve` start, OR the operator explicitly triggers recovery via `quaid collection sync <name> --finalize-pending` (which acquires a short-lived lease and calls the helper as `FinalizeCaller::ExternalFinalize { session_id }`). The command SHALL NOT attempt to rename the target back to staging. Pending state preserved: `state='restoring'`, `pending_root_path=<target>`, `pending_restore_manifest` populated, `restore_command_id` still set while the command is alive (cleared on its exit).
- **Recovery on serve startup or explicit trigger — manifest verified, single-owner gated, integrity-blocking.** Recovery runs ONLY from the serve that has SUCCESSFULLY claimed `collection_owners` for the collection, OR from the `quaid collection sync <name> --finalize-pending` CLI which explicitly acquires a short-lived ownership lease with heartbeat. For every collection where `state = 'restoring'` AND `pending_root_path IS NOT NULL`, the recovery path SHALL:
 - **Stat `pending_root_path`.** If it does NOT exist as a directory → cleanup branch: remove any `<pending_root_path>.quaid-restoring-*/` staging, in one tx revert `state` to `active` (if prior `root_path` readable) or `detached`, NULL `pending_root_path` + `pending_restore_manifest` + `pending_command_heartbeat_at`, clear the ack triple, log `restore_aborted_recovery collection=<N>` at WARN. Done.
 - **Manifest re-validation (MANDATORY before Tx-B).** If `pending_root_path` exists: walk the target directory and verify (a) file count matches `pending_restore_manifest.file_count`; (b) every file's sha256 matches the manifest entry at its relative path; (c) the target root's `fstat(inode, device_id)` matches the manifest's `rename_inode_dev`. ALL three MUST pass. There is NO branch that runs Tx-B from `pending_root_path`'s mere existence — manifest verification is the only gate. The prior "re-execute Tx-B if the target directory exists" rule is formally rejected and enforced against by spec-consistency audit task 17.17.
 - **Verified → Finalize.** All manifest checks passed → run Tx-B idempotently via `run_tx_b` (the single authoritative finalize SQL path per task 17.17(l)). Log `restore_finalize_recovered collection=<N> pending_root_path=<P> manifest_verified=true` at WARN. RCRT's next sweep observes the owned collection with `state='active'` and no live `supervisor_handles` entry, acquires the per-collection single-flight mutex, opens fresh `root_fd` against the new `root_path`, runs `full_hash_reconcile`, starts a new watcher, and spawns a new per-collection supervisor — NO live supervisor observes `restoring → active`; attach is exclusively RCRT's responsibility under the /55/56/57/58/59 single-actor contract.
 - **Verification failed → IntegrityFailed (terminal blocking).** Any mismatch → do NOT run Tx-B. In one tx: keep `state = 'restoring'`, set `integrity_failed_at = now()`, preserve `pending_root_path` and `pending_restore_manifest` for operator inspection. Log `restore_finalize_integrity_failed collection=<N> pending_root_path=<P> files_expected=<N> files_found=<M> first_mismatch=<path>` at ERROR. **Blocking-state predicate:** a FRESH `quaid collection restore` or `sync --remap-root` SHALL refuse with `RestoreIntegrityBlockedError` ONLY when `integrity_failed_at IS NOT NULL` OR `pending_manifest_incomplete_at` has aged past `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS`. Plain `pending_root_path IS NOT NULL` OR `pending_restore_manifest IS NOT NULL` without `integrity_failed_at` is RECOVERABLE, not blocking — a fresh restore invocation returns `RestorePendingFinalizeError` (distinct error) directing the operator to wait or run `sync --finalize-pending`. `sync --finalize-pending` itself SHALL NEVER be refused against a recoverable pending-finalize state — it IS the recovery path, and refusing it would block operator recovery. Only terminal integrity failure (manifest mismatch) or escalated manifest-incomplete requires operator `quaid collection restore-reset <name> --confirm`.
- **Concurrency with user retry.** While `state = 'restoring'` AND `pending_root_path IS NOT NULL`, OR while `integrity_failed_at IS NOT NULL` regardless of `state`, a fresh `quaid collection restore <name> <path>` SHALL error immediately with `RestorePendingFinalizeError` (pending finalize) or `RestoreIntegrityBlockedError` (integrity failure). The command SHALL NOT overwrite the pending intent by running a new staging pass.
- **Idempotency of Tx-B.** Tx-B SHALL be safe to re-execute any number of times while `pending_root_path` is set AND manifest verification passes: each execution writes the same values (including the `needs_full_sync = 1` assignment — which re-arming the write-gate is harmless because RCRT's guarded attach-completion UPDATE `... WHERE needs_full_sync = 1` only clears the flag when set, so idempotent Tx-B calls do not create an observable write-gate flicker). After Tx-B commits `state = 'active'` + NULL `pending_root_path`, subsequent recovery runs find `state = 'active'` and take the no-op path.
- **The collection SHALL be placed in a `restoring` state during the operation to prevent watchers and reconcilers from acting on it.** This applies equally to the pre-rename and pending-finalize windows. Write-interlock (task 11.8) refuses all mutating tools during `state = 'restoring'` OR `needs_full_sync = 1` (write-gate composition — either condition alone triggers `CollectionRestoringError`). The `needs_full_sync = 1` arm covers the post-Tx-B pre-attach window where state has flipped to `'active'` but RCRT has not yet repopulated `file_state`; without this arm, `memory_put` would misclassify every restored page as an `ExternalCreate` (stat succeeds + no `file_state` row → `ConflictError`).

#### Scenario: Happy-path atomic restore — every page byte-exact via raw_imports, two-phase finalize

- **WHEN** a user runs `quaid collection restore work /Users/u/Documents/work-vault-restored` on a detached collection
- **AND** the path `/Users/u/Documents/work-vault-restored` does not exist (or exists as an empty directory)
- **THEN** the system sets `collections.state = 'restoring'` (preventing watcher/reconciler interaction) and NULLs the ack triple (`watcher_released_session_id`, `watcher_released_generation`, `watcher_released_at`) — either via the `--online` handshake opening tx (if online), or via a fresh tx (if offline)
- **AND** creates a sibling staging directory at `/Users/u/Documents/work-vault-restored.quaid-restoring-<uuid>/` (same parent directory as the target so `rename()` stays atomic on a single filesystem)
- **AND** for each page in the collection, writes the bytes from its active `raw_imports` row (`is_active = 1`). Under the v5 invariant this row is present for every page without exception — ingest, reconciler re-ingest, `memory_put` (create AND update), and UUID write-back all rotate raw_imports, and no write path leaves a page without an active row. Restore therefore has exactly one branch: byte-exact recovery from `raw_imports.raw_bytes`.
- **AND** writes `.quaidignore` to the staging directory with the collection's stored ignore patterns
- **AND** verifies: the count of `.md` files written equals the count of active pages in the collection; the sha256 of each written file matches `sha256(raw_imports.raw_bytes)`. Mismatch on any file aborts the restore BEFORE any Tx-A / rename.
- **AND** in **Tx-A** (pre-rename intent + manifest) sets `collections.pending_root_path = '/Users/u/Documents/work-vault-restored'` AND `collections.pending_restore_manifest = <JSON with per-file sha256 + file count + relative paths>` — this is the recoverable "we intend to rename to this target next" signal PLUS the integrity reference the finalize path re-validates
- **AND** if the target is an empty directory, removes it (safe: `rmdir` only succeeds on empty directories); if the target is absent, proceeds directly
- **AND** atomically renames the staging directory to `/Users/u/Documents/work-vault-restored/`, then immediately `fstat`s the target root and updates `pending_restore_manifest` to include `rename_inode_dev = (inode, device_id)` via a brief fresh-connection tx
- **AND** calls `finalize_pending_restore(collection_id, FinalizeCaller::RestoreOriginator { command_id })` where `command_id` is the UUIDv7 token the command generated and wrote to `collections.restore_command_id` at Tx-A (alongside the /85 `restore_command_pid` / `restore_command_host` / `restore_command_start_time_unix_ns` identity tuple, also written at Tx-A) — this caller identity is what authorizes bypass of the fresh-heartbeat defer gate (only the original restore command holds the matching token; a successor serve, a concurrent `sync --finalize-pending`, or a supervisor retry would pass a different caller variant and must defer while the command heartbeat is fresh OR short-circuit via the same-host PID+start-time probe). The helper performs the mandatory manifest re-validation (file count, per-file sha256, `rename_inode_dev` tuple) and on success invokes `run_tx_b` — the **single authoritative finalize SQL path** that atomically clears ALL pending/integrity/originator-identity columns (`pending_root_path = NULL`, `pending_restore_manifest = NULL`, `integrity_failed_at = NULL`, `pending_command_heartbeat_at = NULL`, `restore_command_id = NULL`, `restore_command_pid = NULL`, `restore_command_host = NULL`, `restore_command_start_time_unix_ns = NULL`) AND sets `root_path = <target>`, `state = 'active'`, `needs_full_sync = 1`, bumps `reload_generation`, clears the ack triple, and runs `DELETE FROM file_state WHERE collection_id = ?`. No ad-hoc inline Tx-B SQL is permitted; every RUNTIME finalize path — happy-path restore (`RestoreOriginator`), serve-startup recovery and RCRT sweep (`StartupRecovery`), and `sync --finalize-pending` (`ExternalFinalize`) — routes through `run_tx_b` via `finalize_pending_restore` with an EXPLICIT `FinalizeCaller` variant, enforced by spec-consistency audit task 17.17 (l) and invariant `finalize_pending_restore_caller_explicit` (see task 17.17). **The `FinalizeCaller::SupervisorRetry` variant is NOT a runtime finalize path** under the /55/57 single-actor contract — it is defined in the helper API for test harnesses only. The per-collection supervisor (task 11.7) exits after writing the release ack and does NOT retry finalize; RCRT (task 9.7d) covers all runtime backstop recovery using `StartupRecovery`. The legacy no-arg `finalize_pending_restore(collection_id)` form is forbidden — prior drafts that used it are stale; the caller-scoped form is the only legal API because caller identity determines whether the fresh-heartbeat gate is bypassed or observed. This closes the bug where an inline Tx-B that omitted `pending_restore_manifest = NULL` could self-block future restores after a reported success.
- **AND** returns success with a summary `restored=N byte_exact=N pending_finalize=false` (the two counts are equal by construction under the v5 invariant)

#### Scenario: Tx-B failure — collection recoverable, vault on disk preserved

- **WHEN** the rename has landed but the Tx-B finalize fails (e.g., SQLite busy-timeout reached while another writer holds the WAL, I/O error, or any other SQLite error)
- **THEN** the command SHALL NOT attempt to reverse the rename (the target now holds the restored vault and may already have been opened by other processes)
- **AND** the command does NOT set `collections.needs_full_sync = 1` for this state — Tx-B failure recovery is NOT routed through the generic `needs_full_sync` worker. Reason: the generic worker (task 6.7a / 11.4 equivalent) calls `full_hash_reconcile(collection)` against `collections.root_path`, but at Tx-B failure time `root_path` still points at the OLD vault and `pending_root_path` holds the new target. Running the generic worker here would reconcile the old tree and clear the flag WITHOUT adopting the pending target — exactly the wrong outcome. Recovery MUST go through `finalize_pending_restore(collection_id, FinalizeCaller::RestoreOriginator { command_id })` (the originating command's own retry loop per task 9.7 (l)) OR `FinalizeCaller::StartupRecovery` via RCRT after the command dies — `run_tx_b` is the only path that flips `root_path` to the new target AND runs `full_hash_reconcile` against it via the subsequent RCRT attach. The generic `needs_full_sync` worker SHALL explicitly SKIP any collection with `state = 'restoring'`; it only processes `active` collections (enforced by the worker's `WHERE state = 'active' AND needs_full_sync = 1` query — task 6.7a / 11.4).
- **AND** the command logs `restore_finalize_pending collection=<N> pending_root_path=<P>` at ERROR and exits non-zero with a message informing the user that the vault has been written to the target path and will be finalized on the next `quaid serve` start's RCRT sweep, or immediately via `quaid collection sync <name> --finalize-pending`
- **AND** the collection persists in state `state = 'restoring'` with `pending_root_path = <target>` — this is the recoverable state that RCRT targets on its sweep OR the explicit `--finalize-pending` subcommand targets. When `run_tx_b` eventually commits (via RCRT under `StartupRecovery`, via `sync --finalize-pending` under `ExternalFinalize`, or via the originating command itself under `RestoreOriginator` if still alive), it SETS `needs_full_sync = 1` per the write-gate invariant — this arms the write-interlock until RCRT's subsequent attach-completion tx clears the flag via the guarded UPDATE. Writes are refused in the post-finalize pre-attach window regardless of which caller drove the finalize.

#### Scenario: Recovery on serve startup — originator-dead path, manifest verified, finalize

- **WHEN** `quaid serve` starts, has SUCCESSFULLY claimed `collection_owners` for the collection, and observes `state = 'restoring'` AND `pending_root_path = <target>` AND `<target>` exists as a directory AND `pending_restore_manifest IS NOT NULL`
- **THEN** serve invokes `finalize_pending_restore(collection_id, FinalizeCaller::StartupRecovery { session_id: <own_session_id> })` — NOT an implicit no-caller call. The `StartupRecovery` variant ALWAYS observes the fresh-heartbeat defer gate per task 9.7b / /52 caller-authority rule: only `FinalizeCaller::RestoreOriginator` bypasses the gate, and startup does NOT possess `restore_command_id` (the new serve is a different process from whichever command originally started the restore). The helper evaluates the defer gate BEFORE manifest verification, applying the **/85 same-host PID+start-time short-circuit** first: if `restore_command_host` equals the recovery actor's canonicalized hostname AND `restore_command_pid IS NOT NULL` AND `restore_command_start_time_unix_ns IS NOT NULL`, probe `kill(restore_command_pid, 0)` AND re-read the live process start time. If the probe returns `ESRCH` OR the observed start time diverges from `restore_command_start_time_unix_ns` (PID reuse), the originator is dead — the helper SHORT-CIRCUITS past the wall-clock gate and proceeds to manifest verification immediately, logging `restore_finalize_originator_pid_dead collection=<N> pid=<P> reason=<esrch|pid_reused>` at WARN. Otherwise the wall-clock fallback applies: if `pending_command_heartbeat_at IS NOT NULL AND pending_command_heartbeat_at > now() - (2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS)` (default 60s), a live originator is still refreshing the heartbeat — the helper returns `Deferred` with NO mutation (see adjacent "Recovery deferred on serve startup" scenario). Only when `pending_command_heartbeat_at IS NULL` OR stale (≥ 60s old) OR the PID probe short-circuited does startup proceed to manifest re-validation: (a) walks `<target>` and compares file count against the manifest's expected count; (b) verifies per-file sha256 against the manifest for every entry; (c) stats the target root and compares `(inode, device_id)` against the manifest's recorded `rename_inode_dev` tuple. If ALL three checks PASS → `run_tx_b` is invoked (the single authoritative finalize SQL path per task 17.17(l)) which in one tx sets `root_path = <target>`, NULLs `pending_root_path` / `pending_restore_manifest` / `integrity_failed_at` / `pending_manifest_incomplete_at` / `pending_command_heartbeat_at` / `restore_command_id` / `restore_command_pid` / `restore_command_host` / `restore_command_start_time_unix_ns`, sets `state = 'active'`, SETS `needs_full_sync = 1` (write-gate arming — applies to EVERY `run_tx_b` invocation regardless of caller, so StartupRecovery's recovered-finalize path also arms the gate and closes the post-recovery pre-attach hole), bumps `reload_generation`, clears the ack triple, and `DELETE FROM file_state WHERE collection_id = ?`; log `restore_finalize_recovered collection=<N> pending_root_path=<P> manifest_verified=true caller=StartupRecovery` at WARN; RCRT's next sweep observes `state = 'active' AND needs_full_sync = 1` with no live `supervisor_handles` entry for this collection and invokes the single-flight attach handoff (open fresh `root_fd`, `full_hash_reconcile`, commit the attach-completion tx that clears `needs_full_sync = 0` via the guarded UPDATE `... WHERE needs_full_sync = 1`, start watcher, spawn supervisor, register `JoinHandle`) per task 9.7d's state-transition handoff contract — NOT the pre-"supervisor observes and rebinds" path (the supervisor exits at release per task 11.7 and is no longer polling for state changes). Writes remain refused via the §11.8 OR-composed interlock (`CollectionRestoringError`) throughout the post-recovered-Tx-B pre-attach window until RCRT's attach-completion clears `needs_full_sync = 0`; reads continue to succeed.
- **AND** no user action is required when originator is dead AND manifest verification passes; the collection becomes fully active
- **AND** only the serve that has claimed `collection_owners` for the collection runs this recovery — a second serve that does NOT own the collection SHALL NOT attempt to finalize it

#### Scenario: Recovery deferred on serve startup — originator heartbeat still fresh

- **WHEN** `quaid serve` starts, has SUCCESSFULLY claimed `collection_owners` for the collection, observes `state = 'restoring'` with `pending_root_path` possibly set OR NULL, AND `pending_command_heartbeat_at` is FRESH (within `2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS`, default 60s), AND the /85 same-host PID+start-time probe confirms the originator is genuinely alive (same host + `kill(pid, 0)` succeeds + start time matches `restore_command_start_time_unix_ns`) OR is indeterminate (`restore_command_host` differs, or any identity column is NULL) so the probe cannot prove death
- **THEN** the `FinalizeCaller::StartupRecovery` invocation of `finalize_pending_restore` returns `FinalizeOutcome::Deferred` with NO state mutation and NO manifest re-validation (both gates are evaluated first per task 9.7b, and both must fail to prove the originator dead before deferring). This means the originating restore command is still alive — possibly driving its own `RestoreOriginator`-authorized retry per task 9.7 (l) — and the new serve MUST NOT take over. Log `restore_finalize_deferred_fresh_heartbeat collection=<N> caller=StartupRecovery command_heartbeat_age_s=<A> originator_alive=<probe_result>` at INFO. If the same-host PID+start-time probe instead proved the originator dead (`ESRCH` or start-time mismatch), the helper would SHORT-CIRCUIT to the adjacent "Recovery on serve startup — originator-dead path" scenario immediately rather than waiting for the 60s wall-clock gate to age out.
- **AND** the deferred collection is handled by the Restoring-Collection Retry Task (RCRT, task 9.7d): RCRT is the SOLE runtime actor for `state = 'restoring'` recovery AND for the subsequent `restoring → active` reattach. No per-collection supervisor exists for this collection (the supervisor exits cleanly after writing the release ack per task 11.7's contract and RCRT does not spawn a new one until state transitions to `active`). RCRT re-invokes `finalize_pending_restore(collection_id, FinalizeCaller::StartupRecovery { session_id })` every `QUAID_DEFERRED_RETRY_SECS` (default 30s) until a terminal outcome is reached. When the originator eventually dies, its `pending_command_heartbeat_at` ages past the threshold and the next retry cycle proceeds to manifest verification → `run_tx_b` → the collection transitions to `state = 'active'`. RCRT's NEXT sweep (within another 30s, or in the same sweep after the finalize returns) observes the owned active collection with no live `supervisor_handles` entry, acquires the per-collection single-flight mutex, opens `root_fd`, runs `full_hash_reconcile`, starts a new watcher, spawns a new per-collection supervisor, and registers its `JoinHandle`. No operator intervention or serve restart is required. There is NO "standard supervisor path" that opens `root_fd` independently — attach is exclusively RCRT's responsibility under the single-flight mutex.
- **AND** if the originator is still live when the operator runs `quaid collection sync <name> --finalize-pending` as an external recovery attempt, that CLI also observes the gate (via `FinalizeCaller::ExternalFinalize`) and waits in its own retry loop — per the caller-authority rule, only the originator can bypass. This preserves the invariant across both backstop recovery actors.

#### Scenario: Recovery on serve startup — pending_root_path exists BUT manifest fails → blocking IntegrityFailed

- **WHEN** startup recovery runs against a pending finalize AND the manifest re-validation detects any mismatch (unexpected file count, sha256 drift, or inode/device tuple changed since rename)
- **THEN** serve does NOT run Tx-B. It transitions to `FinalizeOutcome::IntegrityFailed`: in one tx keeps `state = 'restoring'`, sets `collections.integrity_failed_at = now()`, preserves `pending_root_path` and `pending_restore_manifest` for operator inspection; logs `restore_finalize_integrity_failed collection=<N> pending_root_path=<P> files_expected=<N> files_found=<M> first_mismatch=<path>` at ERROR
- **AND** the collection becomes TERMINAL blocking: a fresh `quaid collection restore` or `sync --remap-root` SHALL refuse with `RestoreIntegrityBlockedError` ONLY because `integrity_failed_at IS NOT NULL` (the terminal flag that the `IntegrityFailed` branch just set). Plain `pending_root_path` / `pending_restore_manifest` non-null WITHOUT `integrity_failed_at` is recoverable, NOT blocking — that case returns `RestorePendingFinalizeError` instead and is resolved via RCRT or `sync --finalize-pending`. In THIS scenario `integrity_failed_at` IS set, so the state is terminal. `sync --finalize-pending` SHALL NOT refuse with `RestoreIntegrityBlockedError` on entry — it remains the operator's primary recovery tool. When invoked against this terminal blocking state, `sync --finalize-pending` enters `finalize_pending_restore` which returns `FinalizeOutcome::IntegrityFailed` (the helper does NOT re-run manifest verification needlessly if `integrity_failed_at` is already set from a prior pass); the CLI prints the blocking message and exits non-zero, instructing the operator to run `restore-reset`. The distinction: `RestoreIntegrityBlockedError` is an ENTRY refusal raised by the command guard BEFORE invoking the helper; `FinalizeOutcome::IntegrityFailed` is a helper return code. `sync --finalize-pending` receives the helper return code, never the entry refusal.
- **AND** the operator MUST explicitly clear the terminal blocking state via `quaid collection restore-reset <name> --confirm` after inspecting the pending target directory (per task 9.7e's additional stale-originator-heartbeat gate). Only after that reset is a fresh restore legal.

#### Scenario: Recovery on serve startup — pending_root_path does NOT exist → cleanup

- **WHEN** `quaid serve` starts, has claimed the collection, and observes `state = 'restoring'` AND `pending_root_path = <target>` AND `<target>` does NOT exist on disk (crash happened before rename completed)
- **THEN** serve removes any matching staging directory `<target>.quaid-restoring-*/` (best-effort; partial cleanup is acceptable)
- **AND** in a single tx: NULLs `pending_root_path`, NULLs `pending_restore_manifest`, NULLs `pending_command_heartbeat_at`, NULLs the full originator-identity tuple (`restore_command_id`, `restore_command_pid`, `restore_command_host`, `restore_command_start_time_unix_ns`), clears the ack triple, reverts `state` — to `active` if the prior `root_path` is still a valid, present directory and readable, or to `detached` otherwise
- **AND** logs at WARN: `restore_aborted_recovery collection=<N>`; the user may re-run `quaid collection restore` normally

#### Scenario: Explicit recovery via `sync --finalize-pending` (lease + heartbeat + caller-scoped defer)

- **WHEN** there is NO live `collection_owners` row (either no serve is running, or the prior owning serve died and its row was swept) and a user runs `quaid collection sync <name> --finalize-pending`. Note: a live `collection_owners` row causes immediate `ServeOwnsCollectionError` per task 9.7c — the operator MUST stop serve first (or wait for the owning serve to die) before `sync --finalize-pending` can claim the lease. The "live originator command" case this scenario covers is orthogonal to `collection_owners` liveness: in online-mode restore, the command is a separate process from the serve that holds the lease; if serve dies and `collection_owners` is swept while the originator command is still alive (refreshing `pending_command_heartbeat_at`), the CLI can claim the lease but the helper will still defer to the live originator
- **THEN** the CLI acquires a temporary ownership lease by inserting a `serve_sessions` row and claiming `collection_owners` for the collection (refuses with `ServeOwnsCollectionError` if a live serve owner still exists — that is a distinct precondition from the originator-command liveness below). The CLI spawns a heartbeat task that refreshes `serve_sessions.heartbeat_at` every 5s throughout the helper run so the lease cannot age out during a large-vault manifest walk.
- **AND** the CLI invokes `finalize_pending_restore(collection_id, FinalizeCaller::ExternalFinalize { session_id: <cli_session_id> })` in a retry loop held INSIDE the lease. The `ExternalFinalize` variant ALWAYS observes the fresh-heartbeat defer gate per task 9.7b / caller-authority rule: only `FinalizeCaller::RestoreOriginator { command_id }` bypasses the gate, and the CLI is a distinct process from whichever command originally started the restore (it does NOT possess `restore_command_id`). Before applying the wall-clock gate, the helper applies the **/85 same-host PID+start-time short-circuit** (per task 1.1 and 9.7c): if `restore_command_host` equals the recovery actor's canonicalized hostname AND `restore_command_pid IS NOT NULL` AND `restore_command_start_time_unix_ns IS NOT NULL`, probe `kill(restore_command_pid, 0)` AND re-read the live process start time via the same platform call used at Tx-A capture. If the probe returns `ESRCH` OR the observed start time does not match `restore_command_start_time_unix_ns` (PID reuse), the originator is dead — the helper SHORT-CIRCUITS past the wall-clock gate and proceeds to manifest verification immediately, logging `restore_finalize_originator_pid_dead collection=<N> pid=<P> reason=<esrch|pid_reused>` at WARN. Only when the probe reports the PID alive AND the start time matches (or the host differs, or any identity column is NULL) does the wall-clock fallback apply: if `pending_command_heartbeat_at` is fresh (within `2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS`, default 60s) — this is possible when the originator command is still alive in a separate process even though `collection_owners` (the serve lease) went stale — the helper returns `Deferred` and the CLI retry loop waits `QUAID_RESTORE_RETRY_INTERVAL_SECS` (default 30s) before re-invoking — the CLI does NOT attempt to seize control from a live originator. Only after the heartbeat ages out (the originator died without the PID probe being able to detect it, e.g., cross-host originator) does the `ExternalFinalize` path proceed to manifest verification. This is NOT a contradiction with the `ServeOwnsCollectionError` refusal in step "THEN" above: that refusal gates on `collection_owners` (serve lease) liveness; the defer gate here gates on `pending_command_heartbeat_at` (command process) liveness plus the same-host PID short-circuit; the two can have different truth values in online-mode scenarios where serve died but the command survived.
- **AND** on a terminal outcome the CLI reports and exits: `Finalized` (manifest verified, `run_tx_b` committed, collection now active — exit 0); `IntegrityFailed` (manifest mismatch, blocking state set, operator must run `restore-reset --confirm` — exit non-zero); `Aborted` (pending target missing, cleanup ran — exit 0); `OrphanRecovered` (no pending target but state was stuck — cleanup ran — exit 0); `NothingToDo` (collection not in recoverable state — exit 0). The CLI stops the heartbeat and drops the lease on exit.
- **AND** a `--finalize-pending` invocation against a collection NOT in the pending state returns the `NothingToDo` message with exit 0 — idempotent no-op.
- **AND** if the retry loop reaches `QUAID_FINALIZE_PENDING_TIMEOUT_SECS` (default 3600s) while still `Deferred`, the CLI exits non-zero with a message explaining the heartbeat is still fresh AND the same-host PID-liveness short-circuit could not confirm originator death (either the originator is genuinely alive, runs on a different host, or the identity tuple is incomplete). The message instructs the operator to investigate or wait, and — if the originator is known-dead on a remote host or the PID has been reused in a way the probe can't detect — directs the operator to `quaid collection restore-reset <name> --confirm --force` for explicit manual recovery. This preserves the invariant: ONLY the originator can bypass the heartbeat gate via `RestoreOriginator` identity, and the /85 same-host PID-liveness short-circuit is the ONLY automated external bypass; external callers may never otherwise seize a live restore.

#### Scenario: Explicit blocking-state reset via `restore-reset --confirm`

- **WHEN** a collection is blocked by a TERMINAL integrity state — specifically `integrity_failed_at IS NOT NULL` OR `pending_manifest_incomplete_at IS NOT NULL AND now() - pending_manifest_incomplete_at >= QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` (default 1800s / 30 min) — AND the originator is no longer alive by the /85 combined predicate: EITHER `pending_command_heartbeat_at IS NULL OR pending_command_heartbeat_at <= now() - (2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS)` (wall-clock staleness) OR the same-host PID+start-time probe detects originator death (`restore_command_host` equals the current host AND `kill(restore_command_pid, 0) == ESRCH` OR the live process's start time diverges from `restore_command_start_time_unix_ns` indicating PID reuse), AND the operator runs `quaid collection restore-reset <name> --confirm`
- **THEN** the command acquires the same short-lived lease as `sync --finalize-pending` (refuses if a live serve owner exists with `ServeOwnsCollectionError`), re-checks BOTH the blocking-state predicate AND the combined originator-liveness predicate (wall-clock staleness OR same-host PID short-circuit) INSIDE the lease (TOCTOU defense per task 9.7e step 4), and in one tx NULLs all nine pending/integrity/originator-identity columns (`pending_root_path`, `pending_restore_manifest`, `integrity_failed_at`, `pending_manifest_incomplete_at`, `pending_command_heartbeat_at`, `restore_command_id`, `restore_command_pid`, `restore_command_host`, `restore_command_start_time_unix_ns`), reverts `state` to `active` (if prior `root_path` readable) or `detached` otherwise, and logs `restore_reset_confirmed collection=<N> prior_pending_root_path=<P> prior_integrity_failed_at=<T> prior_manifest_incomplete_at=<T> forced=<bool> pid_shortcircuit=<bool>` at WARN (the two booleans distinguish normal reset, PID-liveness short-circuit reset, and operator `--force` reset for audit)
- **AND** an operator who needs to reset under a same-host live PID that the probe cannot prove dead (e.g., a zombie PID reused by an unrelated process such that `kill(pid, 0)` still succeeds) OR a cross-host originator (where `kill(pid, 0)` is not meaningful) MAY add `--force` to `--confirm` to bypass BOTH the originator-heartbeat gate AND the live-serve-owner gate; the operator is asserting explicit responsibility, and the reset tx logs `restore_reset_forced collection=<N> operator=<user>` at WARN alongside the `forced=true` flag in the standard confirmation log
- **AND** after the reset, the operator may re-run `quaid collection restore` against the same or a different target; the blocking guard is now clear

#### Scenario: `restore-reset --confirm` refuses on non-blocking pending-finalize state

- **WHEN** a collection is in plain `state = 'restoring'` with `pending_root_path` set but `integrity_failed_at IS NULL` and (`pending_manifest_incomplete_at IS NULL` OR still within the escalation window), and the operator runs `quaid collection restore-reset <name> --confirm`
- **THEN** the command refuses with `RestoreResetNotBlockedError` BEFORE any lease acquisition (the gate runs at step 1 of task 9.7e). The error message names the non-blocking state sub-case (plain pending-finalize OR within-escalation-window manifest-incomplete) and instructs the operator to run `quaid collection sync <name> --finalize-pending` or let `quaid serve`'s auto-recovery run, NOT destructive reset. The underlying pending evidence (`pending_root_path`, `pending_restore_manifest`, etc.) is PRESERVED. Reset is reserved for terminal failure states; this scenario proves the tightening.
- **AND** an invocation against a collection with NO pending state (`pending_root_path IS NULL AND pending_manifest_incomplete_at IS NULL AND integrity_failed_at IS NULL`) returns `NothingToReset` with exit code 0 — idempotent no-op, not an error.

#### Scenario: `restore-reset --confirm` refuses while originator heartbeat is fresh

- **WHEN** a collection has `integrity_failed_at IS NOT NULL` (a true terminal blocking state) BUT `pending_command_heartbeat_at` is fresh (< 60s old) AND the /85 same-host PID+start-time probe CANNOT prove the originator dead — i.e., `restore_command_host` differs from the current host, OR any of `restore_command_pid` / `restore_command_start_time_unix_ns` is NULL, OR `kill(restore_command_pid, 0)` returns 0/`EPERM` AND the live process's start time matches `restore_command_start_time_unix_ns` — and the operator runs `quaid collection restore-reset <name> --confirm` (WITHOUT `--force`)
- **THEN** the command refuses with `RestoreResetOriginatorLiveError` naming the remaining seconds the operator must wait before the wall-clock fallback makes reset legal, AND naming the stored `restore_command_pid` / `restore_command_host` so the operator can locate and stop the originator. The originating restore command is genuinely alive (or probed-indeterminate on a remote host) and may be reacting to the integrity failure itself (logging diagnostics, emitting guidance); resetting under a live originator would produce confusing concurrent-actor behavior and destroy evidence the command may still be inspecting.
- **AND** the operator resolves by (a) waiting for the heartbeat to stall (ages out within ~60s of the originator dying; on a same-host originator, the PID-liveness probe will additionally short-circuit the gate as soon as the process exits or is SIGKILLed), (b) sending SIGTERM to the stored `restore_command_pid`, OR (c) — only when the originator is known-dead in a way the probe cannot detect (cross-host origin, or a same-host zombie PID reused by an unrelated process) — re-running `quaid collection restore-reset <name> --confirm --force` to explicitly bypass the gate. `--force` is reserved for this escape hatch; it logs `restore_reset_forced collection=<N> operator=<user>` at WARN and records `forced=true` in the confirmation log so audits can distinguish manual operator overrides from automated recoveries.
- **AND** when the originator eventually dies on the same host (without the operator using `--force`), the PID-liveness probe flips `kill(pid, 0)` to `ESRCH` (or detects a start-time mismatch on a reused PID) at the next reset attempt and the reset proceeds immediately without waiting for the wall-clock heartbeat to age — the confirmation log records `pid_shortcircuit=true`.

#### Scenario: Fresh restore refused while pending finalize OR integrity failure is outstanding

- **WHEN** a collection is in `state = 'restoring'` with `pending_root_path` set (plain recoverable pending-finalize, `integrity_failed_at IS NULL`, `pending_manifest_incomplete_at` within escalation window or NULL) and the user runs `quaid collection restore <same-name> <new-path>`
- **THEN** the command errors immediately with **`RestorePendingFinalizeError`** (a non-blocking, recoverable error) naming `pending_root_path` and instructing the user to either (a) start `quaid serve` for RCRT-driven auto-recovery OR (b) run `quaid collection sync <name> --finalize-pending` to drive finalize explicitly. Operator `restore-reset` is NOT offered as a resolution path because this state is recoverable — invoking reset would destroy the durable evidence of the renamed target.
- **WHEN INSTEAD** a collection has `integrity_failed_at IS NOT NULL` OR `pending_manifest_incomplete_at` aged past `QUAID_MANIFEST_INCOMPLETE_ESCALATION_SECS` (terminal blocking states)
- **THEN** the command errors immediately with **`RestoreIntegrityBlockedError`** naming the specific blocking column and instructing the user to run `quaid collection restore-reset <name> --confirm` after inspecting the pending target. `RestoreIntegrityBlockedError` is ONLY raised for terminal blocking states — a plain recoverable pending-finalize returns `RestorePendingFinalizeError` instead (per the terminal-only predicate)
- **AND** `sync --finalize-pending` SHALL always be allowed against recoverable pending-finalize (it IS the recovery path); it may return `IntegrityFailed`/`Deferred` outcomes per the helper contract but is never itself refused with `RestoreIntegrityBlockedError` on entry
- **AND** no staging directory is created; no mutation occurs

#### Scenario: Tx-B is idempotent under repeated recovery

- **WHEN** Tx-B has partially completed then crashed between SQLite page writes (impossible under SQLite atomic commit semantics, but safety-first for reasoning)
- **THEN** a subsequent recovery re-execution of Tx-B writes the same `(root_path, pending_root_path, state, reload_generation delta, ack triple, file_state cleanup)` values with no divergence
- **AND** after one successful Tx-B commit, subsequent recoveries find `state = 'active'` and take the no-op path (nothing to do)

#### Scenario: Non-empty target rejected

- **WHEN** a user runs `quaid collection restore work /path` and `/path` exists as a non-empty directory, a regular file, a symbolic link, or any other non-empty filesystem entry
- **THEN** the command returns an error naming the target and instructing the user to remove or rename the existing content, or to choose a different target path
- **AND** `collections.state` is NOT changed to `restoring`; no staging directory is created; no mutation occurs

#### Scenario: Empty-directory target accepted

- **WHEN** a user runs `quaid collection restore work /path` and `/path` exists as an empty directory
- **THEN** the system proceeds with staging as described above and removes the empty target directory with `rmdir` immediately before the atomic rename
- **AND** if the `rmdir` fails (e.g., a file was created inside it between the initial check and the rename), the operation aborts, the staging directory is cleaned up, and `collections.state` returns to `detached`

#### Scenario: Restore fails mid-write — collection remains detached, staging cleaned up, no pending intent

- **WHEN** a user runs `quaid collection restore work /path` and a staging write fails partway through (ENOSPC, EACCES, I/O error) — BEFORE Tx-A
- **THEN** the system removes the staging directory (best-effort; partial cleanup is acceptable since it's not at the target path)
- **AND** the SQLite state is reverted: `collections.state` returns to its prior value (`detached`); `collections.root_path` is NOT updated; `pending_root_path` remains NULL (never set because Tx-A never ran)
- **AND** no `file_state` rows are created or deleted
- **AND** the user receives an error identifying the step that failed; on retry the operation runs from scratch

#### Scenario: Restore fails at verification — aborts before Tx-A and rename

- **WHEN** the staging directory is written but verification detects a mismatch (file count wrong, or a sha256 does not match expected) BEFORE Tx-A
- **THEN** the system removes the staging directory, returns an error identifying the specific mismatch
- **AND** `collections.root_path` is unchanged; `collections.state` returns to `detached`; `pending_root_path` remains NULL

#### Scenario: Concurrent restore on the same collection refused

- **WHEN** a restore is in progress (`collections.state = 'restoring'`) and another `quaid collection restore <same-name>...` invocation runs
- **THEN** the second invocation errors immediately with a message identifying the in-progress restore; no staging directory is created

#### Scenario: `quaid serve` handles `restoring` state via the authoritative recovery helper

- **WHEN** `quaid serve` starts up, has SUCCESSFULLY claimed the collection via `collection_owners`, and observes a collection in `restoring` state
- **THEN** serve invokes `finalize_pending_restore(collection_id, FinalizeCaller::StartupRecovery { session_id: <own_session_id> })`. Startup holds lease-holder authority (it just claimed `collection_owners`) but NOT command-identity authority (it does not possess the original `restore_command_id`), so its calls ALWAYS observe the fresh-heartbeat defer gate. The helper branches through the authoritative contract — there is NO branch that runs Tx-B from mere `pending_root_path` existence:
 - **Fresh-heartbeat defer — evaluated FIRST for any non-originator caller**: if `pending_command_heartbeat_at IS NOT NULL` AND `pending_command_heartbeat_at > now() - (2 * QUAID_RELOAD_HANDSHAKE_TIMEOUT_SECS)`, the original restore command may still be alive (its heartbeat is fresh). Return **Deferred** immediately with no mutation. Startup treats the collection as detached this cycle and exits the initial 9.7d pass. The single authoritative retry actor for deferred `restoring` collections is the Restoring-Collection Retry Task (RCRT, task 9.7d), which re-invokes `finalize_pending_restore(..., FinalizeCaller::StartupRecovery)` every `QUAID_DEFERRED_RETRY_SECS` (default 30s) until the heartbeat goes stale OR the command itself transitions state. The per-collection supervisor (task 11.7) does NOT retry `restoring` collections — it only runs while `state = 'active'` — so there is no dual-ownership path between RCRT and the supervisor. When RCRT eventually finalizes/aborts/orphan-recovers, it invokes the supervisor-attach handoff under a per-collection single-flight mutex so exactly one watcher+supervisor starts for the now-active collection.
 - `pending_root_path` NULL, heartbeat stale or NULL → **OrphanRecovered** (revert state to `active` or `detached`, clear ack triple and all pending columns)
 - `pending_root_path` SET, directory does NOT exist → **Aborted** (cleanup staging, revert state, NULL pending columns)
 - `pending_root_path` SET, directory exists, heartbeat stale or NULL → **manifest re-validation is MANDATORY**. Walk the target and compare file count, per-file sha256, and `(inode, device_id)` against `pending_restore_manifest`. ALL three MUST pass to run Tx-B (**Finalized** outcome). ANY mismatch → **IntegrityFailed**: set `integrity_failed_at = now()`, keep `state = 'restoring'`, preserve `pending_root_path` and `pending_restore_manifest` for operator inspection. The collection becomes blocking until `quaid collection restore-reset <name> --confirm` clears the state.
- **AND** `pending_root_path` existing as a directory is NOT sufficient by itself to run Tx-B; the prior "finalize on directory existence" rule is formally rejected and enforced against by spec-consistency audit task 17.17
- **AND** a second `quaid serve` that has NOT claimed the collection via `collection_owners` SHALL NOT invoke the recovery helper for it — the single-owner gate prevents cross-serve interference
- **AND** only the ORIGINAL restore command (presenting `FinalizeCaller::RestoreOriginator { command_id }` matching `collections.restore_command_id`) can bypass the heartbeat defer gate. A successor serve that takes over after the prior serve dies is STILL gated because it does not possess the original command_id.
- **AND** in every branch the supervisor's do-not-impersonate rule (task 11.7a) applies: serve does NOT write the ack triple on behalf of a predecessor session; any pending handshake from a dead session times out on the command side and is cleaned up when the command aborts

#### Scenario: Every page restores byte-exact via raw_imports (no exceptions)

- **WHEN** any page in the collection is restored — regardless of whether the page was originally ingested from disk, authored entirely via `memory_put`, or created by any other code path in v5
- **THEN** the bytes written to disk are identical to the page's active `raw_imports.raw_bytes`
- **AND** the restored file's sha256 equals `sha256(raw_imports.raw_bytes)`
- **AND** formatting details the parser/renderer would normalize (comment markers, unusual whitespace, alternative YAML styles, blockquote indentation, etc.) are preserved exactly as the user originally authored them
- **AND** this applies uniformly: there is no "memory_put-authored pages are re-rendered" branch, no "legacy pages are re-rendered" branch, no "defensive fallback" branch in the normal restore flow — the v5 invariant guarantees every page has an active `raw_imports` row and the restore code has exactly one source of bytes

#### Scenario: Corruption-recovery — missing active raw_imports row aborts restore with diagnostic

- **WHEN** a page unexpectedly has zero active `raw_imports` rows (`SELECT COUNT(*) FROM raw_imports WHERE page_id = ? AND is_active = 1` returns 0) — which SHALL NOT occur under v5 because every content-changing write rotates the row, so this is definitionally a corruption or invariant-violation state
- **THEN** restore SHALL NOT silently fall back to `render_page()` and mask the invariant break. Instead, restore aborts with an `InvariantViolationError` naming the page slug, emits `raw_imports_missing collection=<name> slug=<S> page_id=<id>` at ERROR, and leaves `collections.state = 'detached'` (NO `root_path` change, staging directory removed)
- **AND** the error message instructs the operator to investigate root cause (DB corruption, direct DB manipulation bypassing `rotate_raw_imports`, failed ingest that left `pages` without `raw_imports`) and to run `quaid collection audit` + `quaid collection quarantine list` for diagnostics
- **AND** the operator MAY recover manually by invoking `quaid collection restore <name> <path> --allow-rerender` (an explicit, audit-loggable override flag) that substitutes `render_page()` for any missing rows. This flag SHALL be absent from normal documentation, surfaced only in error messages, and logged at WARN for every page where it fires, so the use of render-as-last-resort is always an explicit user action rather than a silent degrade path

#### Scenario: Restore summary reports recovery quality

- **WHEN** `quaid collection restore` completes successfully
- **THEN** the output includes: total pages restored, count restored byte-exact via `raw_imports`, and the path of any `.quaidignore` file written
- **AND** under the v5 invariant the byte-exact count equals the total restored count; there is no separate re-rendered bucket in the normal flow
- **AND** only if `--allow-rerender` was explicitly passed AND fired on one or more pages does the summary include a `re_rendered=M` line flagged as a corruption-recovery override rather than normal output

#### Scenario: Legacy "materializes vault-authoritative content to disk" expectation

- **WHEN** the atomic restore succeeds
- **THEN** the result is indistinguishable from a single-step write: all pages are materialized at the target path, `.quaidignore` is present, `collections.root_path` is updated, the collection is no longer detached
- **AND** every restored file matches its source bytes — namely `raw_imports.raw_bytes` for the page, per the v5 invariant (no re-rendered branch in the normal flow)

#### Scenario: Restore preserves vault-authoritative fidelity on re-ingest

- **WHEN** a collection is added from vault A, all its pages are exported via `quaid collection restore`, and the restored vault is re-added as a fresh collection in a brand-new memory
- **THEN** every page in the new collection matches the original vault-authoritative state: `compiled_truth`, `timeline`, `frontmatter`, `wing`, `room`, derived `title`, `type`, `summary` are byte-equivalent (up to the normalization performed by the existing roundtrip-semantic test)
- **AND** wiki-link entries in `links` (those extracted from markdown body) are regenerated identically
- **AND** heuristic entries in `assertions` (those produced by `check_assertions()` at ingest) are regenerated identically
- **AND** `page_fts` and `page_embeddings` are regenerated by normal ingest-time triggers and the embedding worker

#### Scenario: Re-ingest into a fresh memory does NOT preserve DB-authoritative state

- **WHEN** a user restores vault A from `memory.db`, then deletes `memory.db`, initializes a fresh memory, and re-adds the restored vault
- **THEN** page markdown content is reconstructed identically
- **AND** programmatic links created via `memory_link` are absent (they were never in the markdown)
- **AND** programmatic assertions created via `memory_check` are absent
- **AND** `raw_data`, `knowledge_gaps`, `contradictions`, and `import_manifest` history are absent
- **AND** this behavior is documented as expected; users who need full fidelity must back up `memory.db` directly

#### Scenario: In-place move preserves all state

- **WHEN** a user moves `memory.db` alongside the vault directory to a new machine and runs `quaid collection sync <name> --remap-root <new-path>` without re-initializing
- **THEN** all DB-authoritative state is present unchanged (because `memory.db` moved with it)
- **AND** vault-authoritative state is validated against the vault on reconciliation (stat-diff + hash-check)
- **AND** no content is lost
