## ADDED Requirements

### Requirement: quaid init command
The `quaid init [PATH]` command SHALL create a new memory database at the given path
(default: platform-appropriate home directory path resolved at runtime). If the file
already exists, the command SHALL print a warning and exit with code 0 without
reinitialising the schema.

#### Scenario: Init at default path
- **WHEN** `quaid init` is run and `~/memory.db` (or platform equivalent) does not exist
- **THEN** a new `memory.db` is created, schema v4 applied, and a success message is printed

#### Scenario: Init at explicit path
- **WHEN** `quaid init /data/work.db` is run
- **THEN** the database is created at `/data/work.db`

#### Scenario: Idempotent init on existing database
- **WHEN** `quaid init` is run and the database already exists
- **THEN** the command prints "Database already exists at <path>" and exits 0 without modification

### Requirement: quaid get command
`quaid get <SLUG>` SHALL read a page from the database by its slug and render it to
stdout as the original compiled_truth + `---` + timeline markdown with frontmatter header.

#### Scenario: Page found
- **WHEN** `quaid get people/alice` is called and the page exists
- **THEN** the full rendered page is printed to stdout and the command exits 0

#### Scenario: Page not found
- **WHEN** `quaid get people/nobody` is called and no such page exists
- **THEN** an error message is printed to stderr and the command exits 1

### Requirement: quaid put command
`quaid put <SLUG>` SHALL read markdown from stdin, parse the frontmatter and content,
auto-derive `wing` and `room`, extract a `summary`, and write/update the page in the
database with OCC enforcement.

#### Scenario: Create new page
- **WHEN** `quaid put people/alice < alice.md` is called and the page does not exist
- **THEN** the page is inserted with `version = 1` and a success message is printed

#### Scenario: Update existing page with correct version
- **WHEN** `quaid put people/alice --expected-version 1 < updated.md` is called
  and the stored version is `1`
- **THEN** the page is updated with `version = 2` and success is reported

#### Scenario: OCC conflict on put
- **WHEN** `quaid put people/alice --expected-version 1 < updated.md` is called
  and the stored version is `2`
- **THEN** an error is printed: "Conflict: page updated elsewhere (current version: 2)" and the command exits 1

#### Scenario: Auto-derive wing from slug
- **WHEN** a page with slug `people/alice-jones` is written
- **THEN** `wing` is set to `"people"` in the stored row

### Requirement: quaid list command
`quaid list` SHALL list all pages in the database with their slug, type, and summary.
`--wing <WING>` SHALL filter by wing. `--type <TYPE>` SHALL filter by page type.
`--limit <N>` SHALL cap results (default: 50).

#### Scenario: List all pages
- **WHEN** `quaid list` is called with a populated database
- **THEN** each page slug, type, and summary is printed one per line, ordered by `updated_at DESC`

#### Scenario: Filter by wing
- **WHEN** `quaid list --wing people` is called
- **THEN** only pages with `wing = 'people'` are returned

### Requirement: quaid stats command
`quaid stats` SHALL print a summary of memory: total pages by type, total links,
FTS5 row count, embedding count, and database file size.

#### Scenario: Stats on populated memory
- **WHEN** `quaid stats` is called with a populated database
- **THEN** a structured summary is printed including at least: total pages, pages by type, total links, embedding count, and file size in MB

### Requirement: quaid tags command
`quaid tags <SLUG> [--add <TAG>] [--remove <TAG>]` SHALL manage tags on a page.
Without flags, it lists current tags. `--add` appends a tag; `--remove` drops it.

#### Scenario: List tags
- **WHEN** `quaid tags people/alice` is called
- **THEN** the current tags for that page are printed, one per line

#### Scenario: Add tag
- **WHEN** `quaid tags people/alice --add investor` is called
- **THEN** `"investor"` is inserted into the `tags` table for this page; the page row version is not incremented

### Requirement: quaid link command
`quaid link <FROM_SLUG> <TO_SLUG> --relationship <REL> [--valid-from <DATE>] [--valid-until <DATE>]`
SHALL create a typed temporal link between two pages.

#### Scenario: Create a link
- **WHEN** `quaid link people/alice companies/acme --relationship works_at --valid-from 2024-01`
  is called
- **THEN** a row is inserted into the `links` table with the specified relationship and validity

#### Scenario: Close a link
- **WHEN** `quaid link people/alice companies/acme --relationship works_at --valid-until 2025-06`
  is called and the link exists
- **THEN** the `valid_until` field of the link is updated

### Requirement: quaid compact command
`quaid compact` SHALL checkpoint the WAL and compact the database to a single file.

#### Scenario: Compact reduces WAL
- **WHEN** `quaid compact` is called after write activity
- **THEN** `PRAGMA wal_checkpoint(TRUNCATE)` executes, the `-wal` sidecar is emptied, and a success message is printed
