## MODIFIED Requirements

### Requirement: Collection as first-class entity

The system SHALL store every page within a named collection. Each collection SHALL have a unique name, a root path on the local filesystem (`collections.root_path TEXT NOT NULL` — storage always retains the last-known absolute path even when the collection is `detached`, so that `sync --remap-root` can be attempted without first re-attaching and operators can see where the collection was last bound), an ignore-pattern list, and a boolean flag indicating whether it is the default target for agent-initiated writes. The MCP surface (`memory_collections`) presents `root_path` as `null` when `state != 'active'` because a detached/restoring root is not safe to walk or open, even though the underlying column is non-null — the API shapes the view, storage retains history.

For first-run usability, `quaid init` SHALL provision the default collection with a writable write-target root at `~/.quaid/vault` (resolved to an absolute path for the current user and created if missing). Existing databases that already have a writable configured write-target root SHALL remain unchanged.

#### Scenario: Fresh init creates default collection

- **WHEN** a user runs `quaid init` in a directory with no existing `memory.db`
- **THEN** the system creates `memory.db`, creates a `collections` row named `default` with `root_path = ~/.quaid/vault` (creating the directory if missing), and sets `is_write_target = 1`

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
