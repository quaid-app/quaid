# fact-extraction-schema Specification

## Purpose
TBD - created by archiving change slm-extraction-and-correction. Update Purpose after archive.
## Requirements
### Requirement: Extracted facts are typed pages with hybrid frontmatter and prose
The system SHALL produce extracted facts as ordinary Quaid pages with `kind` ∈ `{decision, preference, fact, action_item}`. Each fact page SHALL carry common frontmatter fields `kind`, `session_id`, `source_turns` (a list of `<session_id>:<ordinal>` references), `extracted_at` (ISO-8601), `extracted_by` (model alias), `supersedes` (slug of prior head, or null), and `corrected_via` (one of `null`, `'explicit'`, `'file_edit'`). Each kind SHALL additionally carry one or more type-specific structured fields:

- `decision`: `chose` (string, required), `rationale` (string, optional)
- `preference`: `about` (string, required), `strength` (one of `low`, `medium`, `high`, optional)
- `fact`: `about` (string, required)
- `action_item`: `who` (string, optional), `what` (string, required), `status` (one of `open`, `done`, `cancelled`, required), `due` (ISO-8601 date, optional)

The page body SHALL be a prose summary written by the SLM that captures context the structured fields omit (rationale, surrounding circumstance, the conversation snippet that motivates the fact). The structured key (`about` / `chose` / `what`) is the pivot for `fact-resolution`. Both halves matter: structured fields enable cheap dedup/supersede queries; prose enables FTS5 and vector retrieval.

#### Scenario: A preference page has the required structured fields
- **WHEN** the SLM extracts a preference fact from a window
- **THEN** the resulting page's frontmatter contains `kind: preference`, an `about` field, an optional `strength`, the common `session_id`, `source_turns`, `extracted_at`, `extracted_by`, and a non-empty prose body

#### Scenario: A decision page has the required structured fields
- **WHEN** the SLM extracts a decision fact
- **THEN** the resulting page's frontmatter contains `kind: decision`, a `chose` field, optional `rationale`, the common fields, and a non-empty prose body

#### Scenario: An action_item page has the required structured fields with status `open`
- **WHEN** the SLM extracts an action_item fact (newly created)
- **THEN** the resulting page's frontmatter contains `kind: action_item`, a `what` field, `status: open`, optional `who` and `due`, the common fields, and a non-empty prose body

### Requirement: Extracted facts live at `<vault>/extracted/<type>/<slug>.md`
The system SHALL write extracted-fact pages as markdown files in the user's vault at `<vault>/extracted/<type-plural>/<slug>.md`, where `<type-plural>` is one of `decisions`, `preferences`, `facts`, `action-items` and `<slug>` is derived from the type-specific key plus a 4-character collision-avoidance hash (e.g. `matt-prefers-rust-a3f1.md`). Files SHALL be ingested as pages by the existing Phase 4 vault watcher; the worker SHALL NOT write directly to the page table. When namespace isolation is in use, the path SHALL be nested under the namespace directory: `<vault>/<namespace>/extracted/<type-plural>/<slug>.md`.

#### Scenario: A new preference fact creates a markdown file at the canonical path
- **WHEN** the worker accepts a new preference fact with `about: programming-language`
- **THEN** a file is created at `<vault>/extracted/preferences/<slug>.md` (or its namespace-scoped equivalent), the file contains the rendered frontmatter and prose body, and the Phase 4 watcher subsequently ingests it as a page with the correct `kind` and `superseded_by IS NULL`

#### Scenario: Slug collision avoidance produces unique paths
- **WHEN** the worker writes two preference facts with `about: programming-language` whose 4-char hashes happen to differ
- **THEN** both files coexist at distinct paths and both are ingested as separate pages

#### Scenario: The worker does not write directly to the page table
- **WHEN** the worker accepts a new fact and the vault watcher is paused
- **THEN** the file exists on disk but no new page row exists in the database; the page row appears only after the watcher resumes and ingests the file

### Requirement: SLM output contract is JSON-only with per-fact validation
The system's extraction prompt SHALL constrain the SLM to emit a single JSON object of shape `{"facts": [<fact>, ...]}` where each `<fact>` matches the structured field requirements above plus a `summary` field (the prose body). The SLM SHALL NOT emit markdown fences, prose, or commentary outside the JSON object. Empty result SHALL be `{"facts": []}`. The prompt SHALL include an explicit reminder that the model is not a chat partner and SHALL pin at least one simple preference-style output example so short playground-style user statements remain in the JSON contract. The system's parser SHALL: (a) defensively trim leading/trailing whitespace, (b) `serde_json::from_str` into a typed response struct, (c) recover only when exactly one valid `{ "facts": [...] }` object is surrounded by genuinely plain prose commentary, including ordinary prose punctuation such as parentheses or brackets, (d) reject structural wrappers such as markdown fences, XML-ish tags, list markers, extra containers, bracketed/parenthesized envelope wrappers, or multiple top-level objects, and (e) validate each fact against its kind-specific schema, rejecting unknown kinds at the per-fact level while still returning the valid facts from the same response. Whole-response parse failure SHALL count toward `extraction.max_retries`; after the cap is exceeded the queue job SHALL be marked `failed`.

#### Scenario: Single-turn preference window stays inside the JSON envelope
- **WHEN** the worker prompts the SLM for a window whose only new user turn is a direct preference statement such as `I like to drink coffee more than tea`
- **THEN** the prompt still constrains the SLM to return a valid `{"facts":[...]}` or `{"facts":[]}` JSON object
- **AND** the worker does not fail that window solely because the model drifted into chatty non-JSON prose

#### Scenario: Structural wrappers fail closed
- **WHEN** the SLM returns a valid `{"facts":[...]}` envelope wrapped in markdown fences, XML-ish tags, list markers, extra brackets/parentheses, or alongside another top-level JSON object
- **THEN** the parser rejects the whole response instead of unwrapping through that structural wrapper

