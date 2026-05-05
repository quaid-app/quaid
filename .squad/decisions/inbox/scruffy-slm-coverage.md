# Scruffy — SLM coverage gate

- Scope the honest coverage gate to shipped first-slice seams only: schema v9, queue plumbing, conversation capture, and MCP add-turn / close-session surfaces.
- Do not claim extraction-worker, model-lifecycle, or correction-dialogue behavior that is not yet implemented end to end.
- Treat the refreshed `Cargo.lock` entry for `sha1` as required test/coverage infrastructure for the current lane, because stale lock state can make coverage runs fail before any lane tests execute.
