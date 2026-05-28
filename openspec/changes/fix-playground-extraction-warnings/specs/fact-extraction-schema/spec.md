## MODIFIED Requirements

### Requirement: SLM output contract is JSON-only with per-fact validation
The system's extraction prompt SHALL constrain the SLM to emit a single JSON object of shape `{"facts": [<fact>, ...]}` where each `<fact>` matches the structured field requirements above plus a `summary` field (the prose body). The SLM SHALL NOT emit markdown fences, prose, or commentary outside the JSON object. Empty result SHALL be `{"facts": []}`. The prompt SHALL include an explicit reminder that the model is not a chat partner and SHALL pin at least one simple preference-style output example so short playground-style user statements remain in the JSON contract. The system's parser SHALL: (a) defensively trim leading/trailing whitespace, (b) `serde_json::from_str` into a typed response struct, (c) recover only when exactly one valid `{ "facts": [...] }` object is surrounded by genuinely plain prose commentary, including ordinary prose punctuation such as parentheses or brackets, (d) reject structural wrappers such as markdown fences, XML-ish tags, list markers, extra containers, bracketed/parenthesized envelope wrappers, or multiple top-level objects, and (e) validate each fact against its kind-specific schema, rejecting unknown kinds at the per-fact level while still returning the valid facts from the same response. Whole-response parse failure SHALL count toward `extraction.max_retries`; after the cap is exceeded the queue job SHALL be marked `failed`.

#### Scenario: Single-turn preference window stays inside the JSON envelope
- **WHEN** the worker prompts the SLM for a window whose only new user turn is a direct preference statement such as `I like to drink coffee more than tea`
- **THEN** the prompt still constrains the SLM to return a valid `{"facts":[...]}` or `{"facts":[]}` JSON object
- **AND** the worker does not fail that window solely because the model drifted into chatty non-JSON prose

#### Scenario: Structural wrappers fail closed
- **WHEN** the SLM returns a valid `{"facts":[...]}` envelope wrapped in markdown fences, XML-ish tags, list markers, extra brackets/parentheses, or alongside another top-level JSON object
- **THEN** the parser rejects the whole response instead of unwrapping through that structural wrapper
