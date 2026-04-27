# Install Profile Flow — Implementation Checklist

**Scope:** Auto-write PATH and QUAID_DB to shell profile after successful install;
document two-step install for sandboxed environments; match npm postinstall wording.
Closes: #36, #41

---

## Phase A — shell installer profile writes (`scripts/install.sh`)

- [x] A.1 Add `detect_profile` function: inspect `$SHELL` to choose profile file:
  - `*/zsh` → `$HOME/.zshrc`
  - `*/bash` on Darwin → `$HOME/.bash_profile`; on Linux → `$HOME/.bashrc`
  - fallback → `$HOME/.profile`
  Create the file if it does not exist.

- [x] A.2 Add `write_profile_line` helper: takes `(profile, line_to_append)`.
  Checks whether `$profile` already contains the exact `$line_to_append` (using
  `grep -F` fixed-string match); appends it only if not present. Prints
  `  Added <line> to <profile>` when it appends; returns non-zero when already present.

- [x] A.3 Add `write_profile` function: calls `write_profile_line` twice:
  - PATH line: `export PATH="$INSTALL_DIR:$PATH"`
  - QUAID_DB line: `export QUAID_DB="$HOME/memory.db"`
  Prints a final summary line: `Profile updated. Run: source <profile>`

- [x] A.4 Add `--no-profile` flag parsing to the script's argument loop. Treat the
  flag as enabling no-profile mode for that run. Respect existing `QUAID_NO_PROFILE`
  env var.

- [x] A.5 Call `write_profile` (or skip if `QUAID_NO_PROFILE=1`) after the smoke test
  succeeds. Replace the existing `print_path_hint` and `print_quaid_db_tip` calls: when
  profile writes are enabled, the new summary line replaces both; when writes are disabled
  (`--no-profile`), print the existing hint block as fallback.

- [x] A.6 Add the two-step install block to the end of the success output (always printed):
  ```
  For sandboxed/agent environments: download first, then run:
    curl -fsSL .../install.sh -o quaid-install.sh && sh quaid-install.sh
  ```

---

## Phase B — npm postinstall wording

- [x] B.1 Update `packages/quaid-npm/scripts/postinstall.js`: replace the QUAID_DB tip
  wording to match the new shell installer language exactly. No logic change.

---

## Phase C — docs

- [x] C.1 Update `README.md` Quick Start install section: note that the installer writes
  PATH and QUAID_DB to the shell profile automatically and document `QUAID_NO_PROFILE=1`
  for CI/agent users.

- [x] C.2 Update `website/src/content/docs/guides/install.md`: add "Sandboxed / agent
  environments" subsection with two-step install pattern and `QUAID_NO_PROFILE=1`.

- [x] C.3 Update `docs/getting-started.md`: reflect automatic profile setup in the
  post-install walkthrough steps.

---

## Phase D — verification

- [x] D.1 Run `install.sh` in a fresh shell with no existing profile writes. Confirm
  `~/.zshrc` (or appropriate profile) receives both export lines exactly once.

- [x] D.2 Run `install.sh` a second time (upgrade scenario). Confirm no duplicate lines
  are appended to the profile.

- [x] D.3 Run `QUAID_NO_PROFILE=1 sh install.sh`. Confirm no profile writes occur and
  the hint block is printed instead.

- [x] D.4 Run `sh install.sh --no-profile`. Same as D.3.
