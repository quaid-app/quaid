# Mom — parser contract revision

- **Timestamp:** 2026-05-05T17:17:29.932+08:00
- **Change:** `slm-extraction-and-correction`
- **Decision:** Take the narrower truthful path for the parser/window revision. The shipped slice keeps parser-side partial accept for unknown-kind and missing-field facts, and only whole-response parse failures participate in extraction queue retry/fail accounting.
- **Why:** Current code and focused tests already implement per-fact validation error collection plus valid-sibling survival. Extending this slice to fail closed would require new worker behavior and proof tests; leaving the strict retry wording in OpenSpec would over-claim what the batch actually ships.
- **Boundary:** `parse_response()` may return accepted facts plus `validation_errors`; `Worker::infer_and_parse_window()` records queue attempts only when parsing the whole response fails. Future implementation can still tighten this to fail closed, but that is not shipped by this revision.
