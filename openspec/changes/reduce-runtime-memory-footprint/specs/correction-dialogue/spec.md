## ADDED Requirements

### Requirement: Correction dialogue uses the shared SLM runtime
The system SHALL run `memory_correct` and `memory_correct_continue` SLM inference through the same process-wide lazy runtime used by extraction. The correction tools SHALL NOT construct a separate model runner per MCP session, transport connection, or correction dialogue.

#### Scenario: First correction call loads shared runtime
- **WHEN** `memory_correct` is the first SLM-backed call in a process
- **THEN** it loads the configured model alias into the process-wide runtime

#### Scenario: Concurrent correction sessions share loaded runtime
- **WHEN** two MCP sessions run correction dialogues concurrently with the same configured model alias
- **THEN** both dialogues serialize through or otherwise share the same loaded runtime and no second model copy is loaded

#### Scenario: Correction reuses extraction-loaded runtime
- **WHEN** extraction has already loaded the configured model alias
- **THEN** `memory_correct` and `memory_correct_continue` reuse that loaded runtime for SLM inference
