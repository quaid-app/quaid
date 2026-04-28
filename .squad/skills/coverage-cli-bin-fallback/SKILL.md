# Coverage CLI Bin Fallback

Use this when Rust integration tests spawn the repo binary and `cargo llvm-cov` breaks direct `env!("CARGO_BIN_EXE_<name>")` lookup on Windows.

## Pattern

1. Add a tiny shared helper in `tests/common/mod.rs`.
2. First try `option_env!("CARGO_BIN_EXE_quaid")` and accept it only if the file exists.
3. If it is missing, derive fallbacks from `std::env::current_exe()`:
   - sibling `deps\quaid.exe`
   - parent `quaid.exe`
   - parent `deps\quaid.exe`
4. Make every subprocess-style integration test use that helper instead of hardcoding `env!`.
5. Re-run both plain `cargo test` and the coverage command, because the bug usually appears only under `cargo llvm-cov`.

## Good fits in Quaid

- `tests/collection_cli_truth.rs`
- `tests/quarantine_revision_fixes.rs`
- `tests/search_hardening.rs`
