# BGE-small Dual Release Channels — Implementation Checklist

**Scope:** `v0.9.1` dual-release BGE-small channels only — `airgapped` (embedded model bundle) and `online` (runtime-download). No base/large support. No runtime `--model` selector.

---

## Phase A — Compile-time model wiring

- [x] A.1 Confirm the `bge-small-dual-release-channels` OpenSpec change directory exists with all required artifacts: `proposal.md`, `design.md`, `specs/dual-release-assets/spec.md`, `specs/dual-release-install-surface/spec.md`, `specs/dual-release-validation/spec.md`, and this `tasks.md`. Verify the `.openspec.yaml` registration is present and machine-readable.

- [x] A.2 Add a real `embedded-model` Cargo feature and the supporting `build.rs` logic that resolves the BGE-small-en-v1.5 asset bundle (`config.json`, `tokenizer.json`, `model.safetensors`) from a local path or Hugging Face download, copies the files into `OUT_DIR/embedded-model/`, and emits `QUAID_EMBEDDED_CONFIG_PATH`, `QUAID_EMBEDDED_TOKENIZER_PATH`, and `QUAID_EMBEDDED_MODEL_PATH` via `cargo:rustc-env` so the airgapped channel can use `include_bytes!()`.

- [x] A.3 Update `src/core/inference.rs` so the `embedded-model` feature loads model bytes from `include_bytes!()` of the embedded paths, the `online-model` feature retains the download/cache path unchanged, and a `compile_error!` guard prevents both features being enabled simultaneously. The public embedding and search API stays identical across both channels.

- [x] A.4 Set Cargo default features to `["bundled", "embedded-model"]` so `cargo build --release` produces the airgapped binary. The online channel requires explicit `--no-default-features --features bundled,online-model`.

---

## Phase B — Release assets and install surfaces

- [x] B.1 Update `.github/workflows/release.yml` to build and publish both channels for every supported platform. Each platform entry specifies a `channel` (`airgapped` or `online`), the matching `features` flag, and an `artifact` name following `quaid-<platform>-<channel>`. Release includes `.sha256` sidecars for each binary and verifies the full expected asset manifest before publishing.

- [x] B.2 Update `scripts/install.sh` so it defaults to `QUAID_CHANNEL=airgapped` and accepts `QUAID_CHANNEL=airgapped|online`. Both values resolve to `quaid-<platform>-<channel>` asset names. Unknown channel values produce a clear error and exit 1.

- [x] B.3 Update `packages/quaid-npm/scripts/postinstall.js` so it downloads the `online` channel asset (`quaid-<platform>-online`), emits a notice that npm installs the online BGE-small channel and points airgapped users to GitHub Releases or the shell installer. npm always uses the `online` channel; no `QUAID_CHANNEL` override is supported in this surface.

- [x] B.4 Bump version surfaces to `v0.9.1`: `Cargo.toml` `[package].version`, `packages/quaid-npm/package.json` `version`, and any related packaging metadata. Confirm `cargo check` still passes after the bump.

---

## Phase C — Documentation

- [x] C.1 Update `README.md`, `docs/getting-started.md`, and `docs/contributing.md` to describe the two BGE-small release channels accurately: `airgapped` embeds the model bundle (larger binary, zero network required); `online` fetches BGE-small on first semantic use (smaller binary). Document the installer defaults (shell installer → airgapped; npm → online) and state that base/large support and `--model` UX are explicitly deferred.

- [x] C.2 Update website install/user docs (`website/src/content/docs/guides/install.md` and related install pages) so the website describes the dual-channel release story with the correct asset names, installer defaults, and explicit scope boundary (BGE-small only, no `--model` UX).

- [x] C.3 Update technical spec references (`docs/spec.md` and `website/src/content/docs/reference/spec.md`) to replace any single-channel or outdated feature-name language with the dual-channel `airgapped`/`online` release contract.

---

## Phase D — Naming normalization and validation

- [x] D.0 Normalize all implementation-facing contract surfaces (`Cargo.toml` comments, `inference.rs` doc comments, `tasks.md` scope line) to use `airgapped`/`online` channel names only. Remove stale `slim` naming from contract positions. Remaining `slim` in docs prose is descriptive English, not a contract name.

- [x] D.1 Run full repo validation against the completed `v0.9.1` change: `cargo fmt --all --check`, `cargo check`, `cargo test` (all tests passing), and `npm pack --dry-run` from `packages/quaid-npm/` to confirm the online-only package shape. Confirm no reference to `slim` or `quaid-slim-*` remains in code, scripts, or docs.

- [x] D.2 Commit the completed `v0.9.1` dual-release change on `release/v0.9.1-dual-release`, push to remote, and open a PR targeting `main` that references the `bge-small-dual-release-channels` OpenSpec change. Include sign-off that both channel features build cleanly and the asset manifest is correct.
