# Binary & Crate Rename Spec

**Change:** The legacy binary and crate name replaced by `quaid` everywhere in Cargo and the CLI harness.

## Invariants

1. `Cargo.toml` `[package] name` must be `"quaid"`.
2. `[[bin]] name` must be `"quaid"`.
3. `clap` root command `name` attribute must be `"quaid"`.
4. `repository` field must point to `https://github.com/quaid-app/quaid`.
5. The compiled release binary on all platforms must be named `quaid` (or `quaid.exe` on Windows).
6. No legacy binary alias, symlink, or wrapper is created.

## Validation

- `cargo build --release` → binary at `target/release/quaid`.
- `./target/release/quaid --version` exits 0.
- A search for the legacy `name` value in `Cargo.toml` returns zero matches.
