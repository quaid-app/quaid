## ADDED Requirements

### Requirement: gbrain init command
The `gbrain init [PATH]` command SHALL create a new brain database at the given path
(default: platform-appropriate home directory path resolved at runtime). If the file
already exists, the command SHALL print a warning and exit with code 0 without
reinitialising the schema.

#### Scenario: Init at default path
- **WHEN** `gbrain init` is run and `~/brain.db` (or platform equivalent) does not exist
- **THEN** a new `brain.db` is created, schema v4 applied, and a success message is printed

#### Scenario: Init at explicit path
- **WHEN** `gbrain init /data/work.db` is run
- **THEN** the database is created at `/data/work.db`

#### Scenario: Idempotent init on existing database
- **WHEN** `gbrain init` is run and the database already exists
- **THEN** the command prints "Database already exists at <path>" and exits 0 without modification

### Requirement: gbrain get command
`gbrain get <SLUG>` SHALL read a page from the database by its slug and render it to
stdout as the original compiled_truth + `---` + timeline markdown with frontmatter header.

#### Scenario: Page found
- **WHEN** `gbrain get people/alice` is called and the page exists
- **THEN** the full rendered page is printed to stdout and the command exits 0

#### Scenario: Page not found
- **WHEN** `gbrain get people/nobody` is called and no such page exists
- **THEN** an error message is printed to stderr and the command exits 1

### Requirement: gbrain put command
`gbrain put <SLUG>` SHALL read markdown from stdin, parse the frontmatter and content,
auto-derive `wing` and `room`, extract a `summary`, and write/update the page in the
database with OCC enforcement.

#### Scenario: Create new page
- **WHEN** `gbrain put people/alice < alice.md` is called and the page does not exist
- **THEN** the page is inserted with `version = 1` and a success message is printed

#### Scenario: Update existing page with correct version
- **WHEN** `gbrain put people/alice --expected-version 1 < updated.md` is called
  and the stored version is `1`
- **THEN** the page is updated with `version = 2` and success is reported

#### Scenario: OCC conflict on put
- **WHEN** `gbrain put people/alice --expected-version 1 < updated.md` is called
  and the stored version is `2`
- **THEN** an error is printed: "Conflict: page updated elsewhere (current version: 2)" and the command exits 1

#### Scenario: Auto-derive wing from slug
- **WHEN** a page with slug `people/alice-jones` is written
- **THEN** `wing` is set to `"people"` in the stored row

### Requirement: gbrain list command
`gbrain list` SHALL list all pages in the database with their slug, type, and summary.
`--wing <WING>` SHALL filter by wing. `--type <TYPE>` SHALL filter by page type.
`--limit <N>` SHALL cap results (default: 50).

#### Scenario: List all pages
- **WHEN** `gbrain list` is called with a populated database
- **THEN** each page slug, type, and summary is printed one per line, ordered by `updated_at DESC`

#### Scenario: Filter by wing
- **WHEN** `gbrain list --wing people` is called
- **THEN** only pages with `wing = 'people'` are returned

### Requirement: gbrain stats command
`gbrain stats` SHALL print a summary of the brain: total pages by type, total links,
FTS5 row count, embedding count, and database file size.

#### Scenario: Stats on populated brain
- **WHEN** `gbrain stats` is called with a populated database
- **THEN** a structured summary is printed including at least: total pages, pages by type, total links, embedding count, and file size in MB

### Requirement: gbrain tags command
`gbrain tags <SLUG> [--add <TAG>] [--remove <TAG>]` SHALL manage tags on a page.
Without flags, it lists current tags. `--add` appends a tag; `--remove` drops it.

#### Scenario: List tags
- **WHEN** `gbrain tags people/alice` is called
- **THEN** the current tags for that page are printed, one per line

#### Scenario: Add tag
- **WHEN** `gbrain tags people/alice --add investor` is called
- **THEN** `"investor"` is inserted into the `tags` table for this page; the page row version is not incremented

### Requirement: gbrain link command
`gbrain link <FROM_SLUG> <TO_SLUG> --relationship <REL> [--valid-from <DATE>] [--valid-until <DATE>]`
SHALL create a typed temporal link between two pages.

#### Scenario: Create a link
- **WHEN** `gbrain link people/alice companies/acme --relationship works_at --valid-from 2024-01`
  is called
- **THEN** a row is inserted into the `links` table with the specified relationship and validity

#### Scenario: Close a link
- **WHEN** `gbrain link people/alice companies/acme --relationship works_at --valid-until 2025-06`
  is called and the link exists
- **THEN** the `valid_until` field of the link is updated

### Requirement: gbrain compact command
`gbrain compact` SHALL checkpoint the WAL and compact the database to a single file.

#### Scenario: Compact reduces WAL
- **WHEN** `gbrain compact` is called after write activity
- **THEN** `PRAGMA wal_checkpoint(TRUNCATE)` executes, the `-wal` sidecar is emptied, and a success message is printed
