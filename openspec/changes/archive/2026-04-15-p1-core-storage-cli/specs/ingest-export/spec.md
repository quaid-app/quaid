## ADDED Requirements

### Requirement: quaid import command
`quaid import <PATH>` SHALL recursively scan a directory for `.md` files, parse each
file's frontmatter and content, derive `wing`/`room`/`summary` automatically, insert
all pages in a single SQLite transaction with SHA-256-based idempotency, and populate
embeddings after import. A file whose SHA-256 already exists in `ingest_log` SHALL be
skipped silently.

#### Scenario: Import directory of markdown files
- **WHEN** `quaid import /data/memory/` is called on a directory with 100 `.md` files
- **THEN** all 100 pages are inserted in one transaction, embeddings generated, and a
  summary is printed: "Imported 100 pages (0 skipped)"

#### Scenario: Idempotent re-import
- **WHEN** `quaid import /data/memory/` is called a second time with unchanged files
- **THEN** all 100 files are recognised via `ingest_log` SHA-256 match and skipped:
  "Imported 0 pages (100 skipped)"

#### Scenario: Partial re-import after edits
- **WHEN** 5 files are modified and `quaid import /data/memory/` is called again
- **THEN** 5 pages are updated (new SHA-256 triggers re-ingest) and 95 are skipped

#### Scenario: Wing auto-derived from directory structure
- **WHEN** a file at `people/alice.md` is imported
- **THEN** the stored page has `wing = 'people'` and `slug = 'people/alice'`

#### Scenario: Import validate-only mode
- **WHEN** `quaid import /data/memory/ --validate-only` is called
- **THEN** all files are parsed and validated but no database writes are performed;
  any parse errors are printed and the command exits 1 if any errors found

### Requirement: quaid export command
`quaid export <OUTPUT_DIR>` SHALL reconstruct a markdown directory from the database.
Each page SHALL be written as `<output_dir>/<slug>.md` with frontmatter header, compiled_truth,
`---` boundary, and timeline sections. The output SHALL be semantically equivalent to
the input (round-trip safe).

#### Scenario: Export all pages
- **WHEN** `quaid export /tmp/export/` is called
- **THEN** each page in the database is written to a `.md` file with the correct path and content

#### Scenario: Round-trip semantic equivalence
- **WHEN** a corpus is imported and then exported
- **THEN** `quaid import` on the exported directory produces the same database state
  (same page count, same content hashes)

#### Scenario: Export respects slug hierarchy
- **WHEN** a page with slug `companies/acme/products` is exported
- **THEN** it is written to `<output_dir>/companies/acme/products.md` with parent directories created

### Requirement: Round-trip tests
The test suite SHALL include:
1. `tests/roundtrip_semantic.rs` — import a test corpus, export to a temp directory,
   re-import the export, and verify page count and content hashes are identical.
2. `tests/roundtrip_raw.rs` — import a test corpus with `export --raw --import-id`,
   byte-exact diff the output against the source files.

#### Scenario: Semantic round-trip passes
- **WHEN** `roundtrip_semantic` test runs on `tests/fixtures/`
- **THEN** the test passes with zero content hash mismatches

#### Scenario: Raw round-trip passes for canonical input
- **WHEN** `roundtrip_raw` test runs on a canonically-formatted fixture file
- **THEN** the exported file is byte-for-byte identical to the input

### Requirement: quaid ingest command
`quaid ingest <FILE>` SHALL ingest a single source document (article, meeting notes, etc.)
by parsing its frontmatter, deriving metadata, checking SHA-256 idempotency, and storing
it. `--force` SHALL bypass the idempotency check and re-ingest.

#### Scenario: Ingest new document
- **WHEN** `quaid ingest meeting-notes.md` is called and the SHA-256 is not in `ingest_log`
- **THEN** the document is stored and its SHA-256 recorded in `ingest_log`

#### Scenario: Skip duplicate ingest
- **WHEN** `quaid ingest meeting-notes.md` is called and the SHA-256 already exists in `ingest_log`
- **THEN** the command prints "Already ingested (SHA-256 match), use --force to re-ingest" and exits 0

#### Scenario: Force re-ingest
- **WHEN** `quaid ingest meeting-notes.md --force` is called regardless of `ingest_log`
- **THEN** the document is re-ingested and the `ingest_log` entry is updated

### Requirement: Markdown frontmatter parsing
`parse_frontmatter(raw: &str)` SHALL extract a YAML frontmatter block (delimited by `---`
at the start of the file) into a `HashMap` and return the remaining body. The function
SHALL handle files with no frontmatter by returning an empty map.

#### Scenario: Parse valid frontmatter
- **WHEN** `parse_frontmatter("---\ntitle: Alice\ntype: person\n---\n# Alice\n...")` is called
- **THEN** the map contains `title = "Alice"`, `type = "person"`, and the body starts with `# Alice`

#### Scenario: No frontmatter
- **WHEN** `parse_frontmatter("# Just a heading\nSome content")` is called
- **THEN** an empty map is returned and the full string is the body

### Requirement: Compiled_truth / timeline split
`split_content(body: &str)` SHALL split page body at the first line containing only `---`
(after frontmatter) into `(compiled_truth, timeline)` strings.

#### Scenario: Split at boundary
- **WHEN** `split_content("above\n---\nbelow")` is called
- **THEN** `compiled_truth = "above"` and `timeline = "below"`

#### Scenario: No boundary
- **WHEN** `split_content("no boundary here")` is called
- **THEN** `compiled_truth = "no boundary here"` and `timeline = ""`
