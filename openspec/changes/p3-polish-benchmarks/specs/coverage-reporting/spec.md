## ADDED Requirements

### Requirement: Coverage runs for pushes and pull requests targeting main
The CI system SHALL execute a coverage job for pushes to `main` and pull requests targeting `main` using a workflow that is free to run for the repository's public release process.

#### Scenario: Push to main
- **WHEN** a commit is pushed to `main`
- **THEN** the coverage job runs and produces coverage output for that revision

#### Scenario: Pull request to main
- **WHEN** a pull request targets `main`
- **THEN** the coverage job runs for the pull request changes before merge

### Requirement: Coverage output is visible without a paid service
The coverage job SHALL publish both a machine-readable report and a human-readable result through GitHub-hosted or other no-cost public surfaces. Any optional third-party upload SHALL be additive and non-blocking.

#### Scenario: Coverage job succeeds
- **WHEN** the coverage workflow completes successfully
- **THEN** maintainers can inspect a machine-readable coverage artifact and a human-readable summary without requiring a paid dashboard

#### Scenario: Optional third-party upload is unavailable
- **WHEN** an optional external coverage upload fails or is skipped
- **THEN** the workflow still preserves GitHub-visible coverage output and does not fail solely because of the optional upload

### Requirement: Coverage documentation matches the workflow surface
Public docs for release readiness SHALL point to the supported coverage surface and state whether coverage is informational or gating.

#### Scenario: Reader checks coverage instructions
- **WHEN** a contributor reads the README or docs-site coverage guidance
- **THEN** the guidance points to the same coverage output surface produced by CI and does not describe unsupported tooling
