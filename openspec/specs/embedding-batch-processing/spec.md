# embedding-batch-processing Specification

## Purpose
TBD - created by archiving change fix-beta-reported-regressions. Update Purpose after archive.
## Requirements
### Requirement: Embed command processes pages in bounded batches
The `quaid embed` command SHALL process pending pages in bounded batches instead of materializing or embedding the entire corpus in one unbounded operation. The default batch size SHALL be conservative enough for a 350-page BGE-small corpus on macOS arm64 and SHALL be configurable with `--batch-size N`.

#### Scenario: Default batch completes medium corpus
- **WHEN** `quaid embed` runs against a fresh 350-page corpus using the airgapped BGE-small model
- **THEN** the command completes without an OS SIGKILL caused by unbounded first-run memory pressure
- **AND** all non-empty pages have embeddings queued or written according to the existing embed contract

#### Scenario: Operator overrides batch size
- **WHEN** a user invokes `quaid embed --batch-size 25`
- **THEN** the embed command processes at most 25 pages per batch
- **AND** invalid values such as `0` are rejected with a clear CLI error before embedding starts

### Requirement: Batch processing preserves idempotence
Batching SHALL NOT change which pages are embedded, the persisted model metadata, or the behavior of rerunning `quaid embed` after a partial prior run.

#### Scenario: Rerun after partial progress
- **WHEN** a prior embed run wrote embeddings for some pages and exited before completing the corpus
- **THEN** a subsequent `quaid embed` run skips already-current embeddings and completes remaining pages
- **AND** no duplicate current embedding rows are produced

