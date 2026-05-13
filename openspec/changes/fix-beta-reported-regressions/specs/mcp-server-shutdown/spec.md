## ADDED Requirements

### Requirement: MCP shutdown terminates owned runtime work
When `quaid serve` receives SIGTERM or its stdio client disconnects, the system SHALL shut down all Quaid-owned runtime workers and child processes started for that server instance before the parent process exits. Shutdown SHALL NOT terminate unrelated Quaid processes that were not spawned or owned by the server instance.

#### Scenario: SIGTERM leaves no owned children
- **WHEN** a running `quaid serve` process receives SIGTERM
- **THEN** the serve process exits within the shutdown grace period
- **AND** every child process or worker process owned by that serve invocation has exited
- **AND** unrelated Quaid daemon or CLI processes continue running

#### Scenario: Stdio disconnect triggers the same cleanup
- **WHEN** an MCP client closes stdin for `quaid serve`
- **THEN** the server performs the same owned-runtime cleanup as SIGTERM before exiting

### Requirement: Shutdown cleanup is observable in tests
The system SHALL include an integration test that starts the MCP server, records its owned process tree or runtime handles, terminates the server, and verifies the owned work is gone without using broad process-name termination.

#### Scenario: Test uses owned process identity
- **WHEN** the shutdown regression test runs on a supported Unix platform
- **THEN** it identifies the server and owned children by PID or runtime ownership
- **AND** it fails if any owned process remains after SIGTERM
