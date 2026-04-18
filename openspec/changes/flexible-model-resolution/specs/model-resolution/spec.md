## ADDED Requirements

### Requirement: Known alias expansion
The system SHALL resolve short alias strings to canonical HuggingFace model IDs without requiring pinned revision SHAs or file hash verification. Aliases SHALL include: `small`, `base`, `medium`, `large`, `m3`, `max`.

#### Scenario: Small alias resolves correctly
- **WHEN** user passes `--model small` (or omits `--model`)
- **THEN** the system uses model ID `BAAI/bge-small-en-v1.5` with embedding dimension 384

#### Scenario: Base alias resolves correctly
- **WHEN** user passes `--model base`
- **THEN** the system uses model ID `BAAI/bge-base-en-v1.5` with embedding dimension 768

#### Scenario: Medium alias resolves as base
- **WHEN** user passes `--model medium`
- **THEN** the system uses model ID `BAAI/bge-base-en-v1.5` with embedding dimension 768

#### Scenario: Large alias resolves correctly
- **WHEN** user passes `--model large`
- **THEN** the system uses model ID `BAAI/bge-large-en-v1.5` with embedding dimension 1024

#### Scenario: M3 alias resolves correctly
- **WHEN** user passes `--model m3`
- **THEN** the system uses model ID `BAAI/bge-m3` with embedding dimension 1024

#### Scenario: Max alias resolves as m3
- **WHEN** user passes `--model max`
- **THEN** the system uses model ID `BAAI/bge-m3` with embedding dimension 1024

#### Scenario: Full HF model ID accepted silently
- **WHEN** user passes a full HuggingFace model ID (e.g. `--model sentence-transformers/all-MiniLM-L6-v2`)
- **THEN** the system accepts it without warnings and infers embedding dimension at load time

#### Scenario: Full HF ID for known model normalises to alias
- **WHEN** user passes `--model BAAI/bge-base-en-v1.5`
- **THEN** the system resolves it identically to `--model base`

### Requirement: Model list command
The system SHALL provide a `gbrain model list` subcommand that prints a static table of known model aliases, their canonical HuggingFace IDs, embedding dimensions, and approximate download sizes. The command SHALL NOT require network access.

#### Scenario: Plain text output
- **WHEN** user runs `gbrain model list`
- **THEN** a human-readable table is printed to stdout listing all known aliases

#### Scenario: JSON output
- **WHEN** user runs `gbrain model list --json`
- **THEN** a JSON array is printed with fields: `alias`, `model_id`, `dim`, `size_mb`, `notes`

#### Scenario: Help text references model list
- **WHEN** user runs `gbrain --help` or `gbrain init --help`
- **THEN** the `--model` flag description mentions `gbrain model list` for available options
