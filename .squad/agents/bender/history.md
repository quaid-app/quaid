# bender history

- [2026-04-29T07-04-07Z] History summarized and archived
- [2026-04-29T06-55-46Z] Investigated BEIR Regression Gate timeout on PR #114 (release/v0.11.0). Root cause: both beir_nq and beir_fiqa ran the full 10k-doc import+embed+query pipeline before checking whether a baseline existed — both baselines are null/pending in beir.json, so CI burned the entire 60-minute budget every time with no assertion. Fixed by moving the null-baseline early-exit guard to the top of each test function. Committed as 52b46e9, pushed to release/v0.11.0. This is a test-logic fix, not a branch search/embedding regression.
- [2026-04-30T08:30Z] **Batch 4 third-revision cycle complete.** Closed Nibbler's rejection: `mark_collection_restoring_for_handshake` + `wait_for_exact_ack` now use typed `live_collection_owner()` (session_type='serve' enforced) instead of untyped `owner_session_id()` + `session_is_live()`. Removed dead `session_is_live()`. Two tests added (behavioral + source-seam). Clippy clean. 843/843 tests pass. 91.09% line coverage. Committed `714ec48` on `spec/vault-sync-engine-batch4-v0130`. `12.7` remains open (unrelated). Ready for Nibbler re-review.

## Learnings

- [2026-05-27T09:29:59Z] `parse_response` fail-closed recovery has to treat list syntax as structural even when the marker carries annotation text. `- Here is the answer:` and `1. Actual answer:` must reject the wrapped envelope, while plain prose wrappers around exactly one JSON object still recover and the targeted seam stays green with `cargo test --test slm_prompt_parsing --test conversation_turn_capture --quiet`.
- [2026-05-04T07:22:12.881+08:00] Conversation-memory baseline on `feat/slm-conversation-mem`: `cargo llvm-cov report` produced **92.11% TOTAL line coverage** (regions 90.24%, functions 89.06%); `cargo clippy` (default + online), `cargo check`, online-feature tests, `tests/release_asset_parity.sh`, and `tests/install_release_seam.sh` all passed. `tests/install_profile.sh` failed only on the Windows-bash/NTFS unwritable-profile cases (T14/T19/T19c), so treat this workstation as noisy for that seam; the real release blockers are still the unreleased `Cargo.toml` version (`0.17.0`, so `v0.18.0` tagging would fail) and the fact that >90% coverage is a manual gate, not a CI-enforced one.
- [2026-05-04T07:22:12.881+08:00] Conversation-memory coverage panic on `feat/slm-conversation-mem` was not a real >90% regression. The suite was red because `memory_get` returned the sparse stored frontmatter map after updates that omitted `quaid_id`; once the read path re-canonicalized the persisted UUID, `cargo test -j 1` passed (907 lib tests green) and CI-style `cargo llvm-cov --lcov` + `cargo llvm-cov report --summary-only` still measured **92.01% total line coverage / 90.18% total region coverage**.
- [2026-05-04T07:22:12.881+08:00] Add-only supersede chains on rename-before-commit write paths need a real pre-rename semantic claim, not just a preflight head check. The durable fix is to stage the successor row and claim the predecessor inside the same still-open write transaction before any sentinel/tempfile/rename work, then keep the later transactional reconcile as a backstop and prove a losing contender never gets vault bytes or active raw-import ownership onto disk.
## Session: SLM parser validation (2026-05-28)
- Validated Fry's quoted wrapper fix in parser seam
- Focused validation: regression coverage + fail-closed contract
- Verdict: ACCEPT
- Outcome: No blocking objection, ready to merge

## Learnings

- [2026-05-28T03:03:24Z] `line_is_prompt_echo_label` uses `trim_end_matches(':')` (trailing colon only) then normalizes to lowercase/whitespace-collapsed form before exact-matching against the label list. "Example:" and "Example" both match; "Example: some text" does NOT — that's intentional: standalone prompt-echo labels fail closed, labels with explanatory prose still recover.
- [2026-05-28T03:03:24Z] The 3 new single-object wrapper tests (`parse_response_should_reject_single_object_example_wrapper`, `_schema_wrapper`, `_allowed_outputs_wrapper`) cover exactly the seam Fry described. They fail closed as expected. All 37 `slm_prompt_parsing` tests green. Commands: `cargo test --test slm_prompt_parsing --quiet`.
- [2026-05-28T03:03:24.240+00:00] Validation lane validation: reran all 37 parser tests (`tests/slm_prompt_parsing.rs`) for prompt-echo wrapper fail-closed seam. All pass, zero regressions. Confirmed: example/schema/allowed-outputs labels reject, plain prose still recovers. Approved Fry's implementation. Ready to merge.
