## ADDED Requirements

### Requirement: Briefing skill produces "what shifted" report

The briefing skill SHALL generate a structured report covering: pages with recent
`truth_updated_at` or `timeline_updated_at`, newly created pages, unresolved
contradictions, top knowledge gaps, and upcoming timeline entries.

#### Scenario: Daily briefing generation
- **WHEN** an agent reads `skills/briefing/SKILL.md` and follows the workflow
- **THEN** it produces a structured briefing with sections: What Shifted, New Pages,
  Contradictions, Knowledge Gaps, and Upcoming

#### Scenario: Configurable lookback window
- **WHEN** the agent generates a briefing with `--days 7`
- **THEN** only pages changed in the last 7 days appear in "What Shifted"

### Requirement: Alerts skill defines interrupt-driven notification triggers

The alerts skill SHALL define trigger conditions, priority levels, delivery mechanisms,
and deduplication rules for brain state changes.

#### Scenario: New contradiction alert
- **WHEN** `gbrain check --all` detects a new unresolved contradiction
- **THEN** the alerts skill classifies it as high-priority and outputs it

#### Scenario: Stale page alert
- **WHEN** a page has `timeline_updated_at > truth_updated_at` by 30+ days AND has > 5 inbound links
- **THEN** the alerts skill flags it as medium-priority stale risk

### Requirement: Research skill resolves knowledge gaps

The research skill SHALL define a workflow for: fetching unresolved gaps, assessing
sensitivity, generating research queries, ingesting findings, and marking gaps resolved.

#### Scenario: Internal-only research
- **WHEN** a gap has `sensitivity = 'internal'`
- **THEN** the research skill uses only brain-internal search (no external queries)

#### Scenario: Approved external research
- **WHEN** a gap has been approved via `brain_gap_approve` with `sensitivity = 'external'`
- **THEN** the research skill may use Exa or web search with the approved query

### Requirement: Upgrade skill guides binary and skill updates

The upgrade skill SHALL define a workflow for: checking current version, fetching latest
release metadata, downloading and verifying binaries, running post-upgrade validation,
and updating skills.

#### Scenario: Version check
- **WHEN** the agent runs `gbrain version`
- **THEN** the upgrade skill compares against the latest GitHub Release

#### Scenario: Post-upgrade validation
- **WHEN** a new binary is installed
- **THEN** the upgrade skill runs `gbrain validate --all` to confirm DB compatibility

### Requirement: Enrichment skill patterns for external data

The enrichment skill SHALL define integration patterns for: Crustdata (company/person data),
Exa (web search/extraction), and Partiful (event/social data). It SHALL document how
enrichment data flows into `raw_data` table and how facts are extracted into
`compiled_truth` and `assertions`.

#### Scenario: Crustdata enrichment
- **WHEN** the agent enriches a company page
- **THEN** it uses `brain_raw` to store Crustdata response and updates compiled_truth

### Requirement: Skills doctor command

`gbrain skills doctor` SHALL display the active skill resolution order (embedded defaults →
`~/.gbrain/skills/` → working directory `./skills/`), content hashes (SHA-256) for each
resolved skill file, and version compatibility notes.

#### Scenario: Default resolution
- **WHEN** no external skills exist
- **THEN** `skills doctor` shows only embedded skills with their content hashes

#### Scenario: Override detection
- **WHEN** `./skills/ingest/SKILL.md` exists in the working directory
- **THEN** `skills doctor` shows the override and flags the embedded version as shadowed
