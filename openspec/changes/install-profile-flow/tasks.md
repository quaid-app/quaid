# Install Profile Flow — Implementation Checklist

**Scope:** Auto-write PATH and GBRAIN_DB to shell profile after successful install;
document two-step install for sandboxed environments; match npm postinstall wording.
Closes: #36, #41

---

## Phase A — shell installer profile writes (`scripts/install.sh`)

- [ ] A.1 Add `detect_profile` function: inspect `$SHELL` to choose profile file:
  - `*/zsh` → `$HOME/.zshrc`
  - `*/bash` on Darwin → `$HOME/.bash_profile`; on Linux → `$HOME/.bashrc`
  - fallback → `$HOME/.profile`
  Create the file if it does not exist.

- [ ] A.2 Add `write_profile_line` helper: takes `(profile, marker_pattern, line_to_append)`.
  Greps `$profile` for `$marker_pattern`; appends `$line_to_append` only if not present.
  Prints `  Added <line> to <profile>` when it appends; silently skips when already present.

- [ ] A.3 Add `write_profile` function: calls `write_profile_line` twice:
  - PATH: marker `$INSTALL_DIR`, line `export PATH="$INSTALL_DIR:$PATH"`
  - GBRAIN_DB: marker `GBRAIN_DB`, line `export GBRAIN_DB="$HOME/brain.db"`
  Prints a final summary line: `Profile updated. Run: source <profile>`

- [ ] A.4 Add `--no-profile` flag parsing to the script's argument loop. Set
  `GBRAIN_NO_PROFILE=1` when the flag is present. Respect existing `GBRAIN_NO_PROFILE` env var.

- [ ] A.5 Call `write_profile` (or skip if `GBRAIN_NO_PROFILE=1`) after the smoke test
  succeeds. Replace the existing `print_path_hint` and `print_gbrain_db_tip` calls: when
  profile writes are enabled, the new summary line replaces both; when writes are disabled
  (`--no-profile`), print the existing hint block as fallback.

- [ ] A.6 Add the two-step install block to the end of the success output (always printed):
  ```
  For sandboxed/agent environments: download first, then run:
    curl -fsSL .../install.sh -o gbrain-install.sh && sh gbrain-install.sh
  ```

---

## Phase B — npm postinstall wording

- [ ] B.1 Update `packages/gbrain-npm/scripts/postinstall.js`: replace the GBRAIN_DB tip
  wording to match the new shell installer language exactly. No logic change.

---

## Phase C — docs

- [ ] C.1 Update `README.md` Quick Start install section: note that the installer writes
  PATH and GBRAIN_DB to the shell profile automatically and document `GBRAIN_NO_PROFILE=1`
  for CI/agent users.

- [ ] C.2 Update `website/src/content/docs/guides/install.md`: add "Sandboxed / agent
  environments" subsection with two-step install pattern and `GBRAIN_NO_PROFILE=1`.

- [ ] C.3 Update `docs/getting-started.md`: reflect automatic profile setup in the
  post-install walkthrough steps.

---

## Phase D — verification

- [ ] D.1 Run `install.sh` in a fresh shell with no existing profile writes. Confirm
  `~/.zshrc` (or appropriate profile) receives both export lines exactly once.

- [ ] D.2 Run `install.sh` a second time (upgrade scenario). Confirm no duplicate lines
  are appended to the profile.

- [ ] D.3 Run `GBRAIN_NO_PROFILE=1 sh install.sh`. Confirm no profile writes occur and
  the hint block is printed instead.

- [ ] D.4 Run `sh install.sh --no-profile`. Same as D.3.
