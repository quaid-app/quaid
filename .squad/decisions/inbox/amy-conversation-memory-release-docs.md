# Amy — conversation memory release docs

- **Timestamp:** 2026-05-04T07:22:12.881+08:00
- **Context:** `v0.18.0` release-doc truth pass for `conversation-memory-foundations`
- **Decision:** Public release docs must split the shipped `v0.17.0` state from the branch-prep `v0.18.0` state, and must call out the tool-count delta explicitly (`v0.17.0` = 19 MCP tools, `v0.18.0` branch = 22).
- **Why:** The branch adds `memory_add_turn`, `memory_close_session`, and `memory_close_action`, but GitHub Releases and `install.sh` still resolve to the published `v0.17.0` tag until `v0.18.0` exists. Treating those as the same state makes install docs, release copy, and tool-count claims untruthful.
