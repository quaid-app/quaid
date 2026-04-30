---
name: "frozen-public-contracts"
description: "How to ship diagnostics without breaking a frozen MCP/JSON contract"
domain: "release-gating, API contracts"
confidence: "high"
source: "earned — vault-sync-engine Batch 1/2 repairs"
---

## Context

Use this when an OpenSpec slice says a public surface is frozen (for example, an MCP tool with an exact-key test) but a new batch wants to expose more diagnostics. This pattern is for Quaid's mixed CLI + MCP surfaces, especially when `src/mcp/server.rs` has schema-locking tests.

## Patterns

### 1. Treat exact-key tests as hard contract locks

If a test asserts that a JSON object has exactly N keys, the surface is frozen. Do not add fields "just for this batch" unless the OpenSpec explicitly reopens the contract, updates design/spec artifacts, and updates the exact-key test in the same patch.

### 2. Prefer CLI-only widening before MCP widening

When operators need more diagnostics urgently, add them to `quaid collection info` (plain text and `--json`) instead of widening frozen MCP output. CLI widening is still a public contract, but it avoids breaking agents pinned to the MCP schema.

### 3. Plain text and `--json` are separate truth surfaces

Do not mark a task closed if only the human-readable CLI output shows the new field. A `#[serde(skip_serializing)]` or similar omission means the JSON contract still lacks the feature.

### 4. Rewrite task wording when the accepted contract narrows

If the implementation plan originally says "surface X in MCP + CLI" but reviewer-approved artifacts later freeze MCP, update `implementation_plan.md` and `tasks.md` to the narrowed contract before closing the task. A green checkbox must describe the accepted contract, not the superseded draft.

## Examples

- `memory_collections` frozen at 13 fields in task 13.6 + exact-key test in `src/mcp/server.rs`
- Batch 2 failed-job diagnostics shipped via `quaid collection info` while MCP stayed unchanged
- `tests/collection_cli_truth.rs` updated so `collection info --json` proves the field is truly surfaced

## Anti-Patterns

- Adding a "small" MCP field to a frozen response without reopening the spec
- Claiming a task is done because internal code computes a value that no public surface exposes
- Surfacing a field in plain text only and forgetting `--json`
