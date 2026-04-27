## ADDED Requirements

### Requirement: Semantic similarity gate before cross-page assertion comparison
**Condition:** This requirement is gated — implement Phase E (semantic gate) only if
`check --all` still produces a material false-positive rate after landing Phase A
(extraction tightening). Rerun the benchmark corpus before starting Phase E work.

Before comparing `is_a` assertions between two pages, the system SHALL compute cosine
similarity between those pages' stored embeddings. If cosine similarity is below the
configured floor (`assertion_similarity_floor`, default `0.6`), the pair SHALL be skipped.

#### Scenario: Unrelated pages skipped by similarity gate
- **WHEN** page A (about retrieval algorithms) and page B (about a CLI tool) have
  cosine similarity 0.12
- **THEN** their `is_a` assertions are not compared and no contradiction is emitted

#### Scenario: Related pages pass the gate
- **WHEN** page A and page B both describe the same entity and have cosine similarity 0.82
- **THEN** their `is_a` assertions are compared normally and any genuine contradiction fires

#### Scenario: Pages without embeddings fall through to existing behavior
- **WHEN** one of the pages has no stored embedding vector
- **THEN** the system falls back to comparing assertions anyway (fail-open), with a debug log

#### Scenario: Configurable threshold respected
- **WHEN** `config` table key `assertion_similarity_floor` is set to `0.0`
- **THEN** all page pairs are compared regardless of embedding similarity (restores old behavior)
