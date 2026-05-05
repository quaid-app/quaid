## ADDED Requirements

### Requirement: `correction_sessions` table backs the bounded dialogue
The system SHALL provide a SQLite-backed `correction_sessions` table with columns `correction_id TEXT PRIMARY KEY`, `fact_slug TEXT NOT NULL`, `exchange_log TEXT NOT NULL` (JSON array of `{role, content}` exchanges), `turns_used INTEGER NOT NULL DEFAULT 0`, `status TEXT NOT NULL CHECK (status IN ('open', 'committed', 'abandoned', 'expired'))`, `created_at TEXT NOT NULL`, and `expires_at TEXT NOT NULL`. A partial index `idx_correction_open ON correction_sessions(status, expires_at) WHERE status = 'open'` SHALL exist to make the janitor's expiry sweep cheap. Session expiry SHALL default to 1 hour after `created_at`.

#### Scenario: Fresh v9 schema includes the table and index
- **WHEN** `quaid init` creates a fresh v9 database
- **THEN** the `correction_sessions` table exists with the documented columns and CHECK constraints, and `idx_correction_open` exists with its partial-index `WHERE` clause

### Requirement: `memory_correct` opens a bounded correction dialogue
The system SHALL expose an MCP tool `memory_correct` with input `{fact_slug: string, correction: string}`. On invocation, the system SHALL: (a) verify `fact_slug` resolves to a head page of `kind ∈ {decision, preference, fact, action_item}`, (b) create a new `correction_sessions` row with `status: open`, `expires_at: now + 1h`, `turns_used: 1`, and `exchange_log: [{role: "user", content: <correction>}]`, (c) invoke the SLM with the correction-mode prompt, (d) return either `{status: "committed", new_fact_slug, supersedes}` (the SLM produced a confident corrected fact and it was written via the supersede path) or `{status: "needs_clarification", correction_id, question, turns_remaining}`. The tool SHALL return `KindError` if the fact is not one of the four extractable kinds, `NotFoundError` if the slug does not resolve, and `ConflictError` if the resolved page is non-head (already superseded).

#### Scenario: One-shot correction commits without dialogue
- **WHEN** `memory_correct` is called with a clear correction the SLM can act on confidently
- **THEN** a new fact page is written with `supersedes: <fact_slug>` and `corrected_via: explicit`, the correction session row is updated to `status: committed`, and the response is `{status: "committed", new_fact_slug, supersedes}`

#### Scenario: Ambiguous correction returns a clarification question
- **WHEN** `memory_correct` is called with a correction the SLM judges ambiguous (e.g. "could be either A or B")
- **THEN** no fact is written yet, the response is `{status: "needs_clarification", correction_id, question, turns_remaining: 2}`, and the correction session row remains `status: open` with `turns_used: 1` and the exchange log including the SLM's clarifying question

#### Scenario: Non-head fact rejects correction
- **WHEN** `memory_correct` is called with a `fact_slug` whose page is non-head (already superseded)
- **THEN** the tool returns `ConflictError` with a message instructing the caller to correct the current head instead

### Requirement: `memory_correct_continue` advances or abandons the dialogue
The system SHALL expose an MCP tool `memory_correct_continue` with input `{correction_id: string, response?: string, abandon?: bool}`. Exactly one of `response` and `abandon: true` SHALL be set per call. On `response`: the system SHALL append `{role: "user", content: <response>}` to the exchange log, increment `turns_used`, invoke the SLM with the full exchange context, and return one of the same shapes as `memory_correct` (`committed` | `needs_clarification` | `abandoned`). On `abandon: true`: the system SHALL update `status: abandoned` and return `{status: "abandoned", reason: "user_requested"}` without invoking the SLM.

#### Scenario: Continued response commits a corrected fact
- **WHEN** `memory_correct_continue` is called with the user's clarifying response and the SLM returns a confident corrected fact
- **THEN** a new fact page is written with `supersedes: <original_fact_slug>` and `corrected_via: explicit`, the correction session row becomes `status: committed`, and the response is `{status: "committed", new_fact_slug, supersedes}`

#### Scenario: Explicit abandon ends the dialogue without writing a fact
- **WHEN** `memory_correct_continue` is called with `abandon: true`
- **THEN** the SLM is not invoked, the correction session row becomes `status: abandoned`, no fact page is written, and the response is `{status: "abandoned", reason: "user_requested"}`

#### Scenario: Unknown correction_id returns NotFoundError
- **WHEN** `memory_correct_continue` is called with a `correction_id` that does not exist
- **THEN** the tool returns `NotFoundError` and no state mutation occurs

### Requirement: Dialogue is hard-capped at 3 SLM exchanges
After the third SLM invocation in a correction session (counting the initial `memory_correct` call as exchange 1), the dialogue SHALL be terminated. If the third exchange does not produce a confident commit, the session SHALL transition to `status: abandoned` with `reason: "turn_cap_reached"` and the response SHALL be `{status: "abandoned", reason: "turn_cap_reached"}`.

#### Scenario: Three clarifications without commit forces abandon
- **WHEN** the SLM returns `needs_clarification` on exchanges 1 and 2, and on exchange 3 still does not produce a confident commit
- **THEN** the session transitions to `status: abandoned`, `reason: turn_cap_reached`, no fact is written, and subsequent `memory_correct_continue` calls on the same `correction_id` return `ConflictError`

### Requirement: Correction-mode SLM prompt is constrained to three outcomes
The SLM correction-mode prompt SHALL constrain the model to produce one of three structured outputs per turn: (a) a `commit` containing the corrected fact in the canonical hybrid frontmatter + prose shape (matching `fact-extraction-schema`); (b) a `clarify` containing exactly one question for the user; (c) an `abandon` containing a short reason. The prompt SHALL emphasise that the SLM is not a chat partner — its job is to determine the corrected fact and write it.

#### Scenario: Commit output produces a corrected fact via the supersede path
- **WHEN** the SLM emits a `commit` output with a corrected fact
- **THEN** the output is parsed using the same strict JSON contract as the extraction prompt, the resulting fact is written via the supersede path (proposal #1's `add-only-supersede-chain`), and `corrected_via: explicit` is set on the new head's frontmatter

#### Scenario: Clarify output advances the dialogue
- **WHEN** the SLM emits a `clarify` output with a question
- **THEN** the question is returned to the caller in the `needs_clarification` response and the session waits for `memory_correct_continue`

#### Scenario: SLM-driven abandon ends the dialogue
- **WHEN** the SLM emits an `abandon` output (deciding the correction is not actionable)
- **THEN** the session transitions to `status: abandoned` with `reason: "slm_abandoned"`, no fact is written, and the response is `{status: "abandoned", reason: "slm_abandoned"}`

### Requirement: Expired open sessions are swept hourly
The hourly janitor SHALL transition `correction_sessions` rows where `status = 'open' AND expires_at < now()` to `status = 'expired'`. Subsequent `memory_correct_continue` calls referencing an expired `correction_id` SHALL return `ConflictError` with a clear message instructing the caller to start a new correction. (The janitor itself is owned by `extraction-worker`; this requirement defines the correction-side contract the janitor satisfies.)

#### Scenario: Hour-old open session is marked expired
- **WHEN** the janitor runs and a `correction_sessions` row has `status = 'open'` with `expires_at` 1 minute past
- **THEN** the row's status becomes `expired`

#### Scenario: Continuation on an expired session returns ConflictError
- **WHEN** `memory_correct_continue` is called with a `correction_id` whose status is `expired`
- **THEN** the tool returns `ConflictError` and no SLM invocation occurs
