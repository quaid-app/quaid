# Scruffy — conversation-memory slice 2 test decision

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Scope:** `conversation-memory-foundations` tasks 2.2-2.5 / 3.1-3.7
- **Decision:** Treat the existing text-query supersede integration as necessary but insufficient. Keep dedicated proofs for exact-slug head filtering, progressive expansion refusing superseded neighbours by default, and graph traversal surfacing `superseded_by` edges distinctly.
- **Why:** Those branches are where this slice can look covered while still lying: exact-slug query paths bypass the generic recall path, progressive retrieval can accidentally reintroduce historical pages during expansion, and graph traversal needs its own proof that supersede edges are first-class.
- **Coverage note:** Current honest coverage is far below 90% for the branch, so this slice should be reported truthfully rather than treated as "covered enough" by the existing broad suite.
