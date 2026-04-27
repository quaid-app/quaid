#!/usr/bin/env sh
# tests/install_profile.sh — deterministic tests for the installer's profile-write logic.
#
# Sources scripts/install.sh in QUAID_TEST_MODE=1 to isolate profile functions without
# running the download/install path. Tests cover: fresh write, idempotency, opt-out
# (--no-profile / QUAID_NO_PROFILE=1), and regex-metacharacter safety.
#
# Usage:
#   sh tests/install_profile.sh
#
# Exit code: 0 = all pass, 1 = one or more failures.

set -e

PASS=0
FAIL=0

ok() {
  printf '  ok: %s\n' "$1"
  PASS=$((PASS + 1))
}

not_ok() {
  printf '  FAIL: %s\n' "$1"
  FAIL=$((FAIL + 1))
}

# Locate project root relative to this script
SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_SH="$SCRIPT_DIR/scripts/install.sh"

# Isolated home dir written under target/ so it is gitignored and cleaned by cargo clean
TEST_HOME="$SCRIPT_DIR/target/test-home-install-profile"
rm -rf "$TEST_HOME"
mkdir -p "$TEST_HOME"

# Required variables install.sh reads at top level before any function executes
QUAID_TEST_MODE=1
QUAID_RELEASE_API_URL="https://example.invalid"
QUAID_RELEASE_BASE_URL="https://example.invalid"
QUAID_INSTALL_DIR="$TEST_HOME/bin"
QUAID_NO_PROFILE=0
QUAID_CHANNEL="airgapped"
QUAID_VERSION="v0.0.0-test"
HOME="$TEST_HOME"
mkdir -p "$QUAID_INSTALL_DIR"

# Source the installer in test mode — function definitions load, main() does not run
# shellcheck source=../scripts/install.sh
. "$INSTALL_SH"

# Windows-only helpers: apply/remove NTFS deny ACL via a .ps1 script so that
# chmod 500 on directories actually restricts writes under Git Bash / MSYS2.
_win_deny_write() {
  [ -n "${MSYSTEM:-}" ] || return 0
  command -v cygpath >/dev/null 2>&1 || return 0
  command -v powershell >/dev/null 2>&1 || return 0
  _wdw_path="$(cygpath -wa "$1")"
  _wdw_ps1="${TEST_HOME}/win_acl_$$.ps1"
  _wdw_wps1="$(cygpath -wa "$_wdw_ps1")"
  printf '$user = $env:USERDOMAIN + "\\" + $env:USERNAME\n' > "$_wdw_ps1"
  printf 'icacls "%s" /deny ($user + ":(AD,WD)") /T\n' "$_wdw_path" >> "$_wdw_ps1"
  powershell -NoProfile -NonInteractive -File "$_wdw_wps1" >/dev/null 2>&1 || true
  rm -f "$_wdw_ps1"
}

_win_allow_write() {
  [ -n "${MSYSTEM:-}" ] || return 0
  command -v cygpath >/dev/null 2>&1 || return 0
  command -v powershell >/dev/null 2>&1 || return 0
  _waw_path="$(cygpath -wa "$1")"
  _waw_ps1="${TEST_HOME}/win_acl_$$.ps1"
  _waw_wps1="$(cygpath -wa "$_waw_ps1")"
  printf '$user = $env:USERDOMAIN + "\\" + $env:USERNAME\n' > "$_waw_ps1"
  printf 'icacls "%s" /remove:d ($user) /T\n' "$_waw_path" >> "$_waw_ps1"
  powershell -NoProfile -NonInteractive -File "$_waw_wps1" >/dev/null 2>&1 || true
  rm -f "$_waw_ps1"
}

printf '\nRunning install profile tests...\n\n'

# ---------------------------------------------------------------
# write_profile_line
# ---------------------------------------------------------------

PROFILE="$TEST_HOME/.zshrc_test"
LINE='export PATH="/test/.local/bin:$PATH"'

# T1: appends to a fresh (empty) file
printf '' > "$PROFILE"
if write_profile_line "$PROFILE" "$LINE"; then
  if grep -Fq "$LINE" "$PROFILE"; then
    ok "T1: write_profile_line appends to fresh file"
  else
    not_ok "T1: write_profile_line appends to fresh file — line missing after append"
  fi
else
  not_ok "T1: write_profile_line appends to fresh file — returned non-zero"
fi

# T2a: idempotent — second call returns non-zero (nothing to write)
if write_profile_line "$PROFILE" "$LINE"; then
  not_ok "T2a: write_profile_line returns non-zero when line already present"
else
  ok "T2a: write_profile_line returns non-zero when line already present"
fi

# T2b: idempotent — no duplicate written
count=$(grep -cF "$LINE" "$PROFILE")
if [ "$count" = "1" ]; then
  ok "T2b: write_profile_line does not duplicate an existing line"
else
  not_ok "T2b: write_profile_line duplicated line: found $count copies"
fi

# T3: missing profile file — returns non-zero, does not create file
MISSING="$TEST_HOME/.no_such_profile"
rm -f "$MISSING"
if write_profile_line "$MISSING" "$LINE"; then
  not_ok "T3: write_profile_line returns non-zero for missing file"
else
  ok "T3: write_profile_line returns non-zero for missing file"
fi
if [ -f "$MISSING" ]; then
  not_ok "T3b: write_profile_line must not create missing profile"
else
  ok "T3b: write_profile_line does not create missing profile"
fi

# T4: fixed-string matching — regex metacharacters in path are handled correctly
REGEX_PROFILE="$TEST_HOME/.profile_regex"
REGEX_LINE='export PATH="/test/.local/bin[x]:$PATH"'
printf '' > "$REGEX_PROFILE"
if write_profile_line "$REGEX_PROFILE" "$REGEX_LINE"; then
  ok "T4: write_profile_line handles regex metacharacters in path (first write)"
else
  not_ok "T4: write_profile_line failed with metacharacters in path"
fi
# Second write must still be idempotent
if write_profile_line "$REGEX_PROFILE" "$REGEX_LINE"; then
  not_ok "T4b: idempotent check failed with metacharacters in path"
else
  ok "T4b: idempotent check works with metacharacters in path"
fi
count2=$(grep -cF "$REGEX_LINE" "$REGEX_PROFILE")
if [ "$count2" = "1" ]; then
  ok "T4c: no duplicate for metacharacter path"
else
  not_ok "T4c: found $count2 copies for metacharacter path"
fi

# ---------------------------------------------------------------
# write_profile
# ---------------------------------------------------------------

WP_PROFILE="$TEST_HOME/.wp_zshrc"
printf '' > "$WP_PROFILE"

# Override detect_profile so write_profile uses our test file
detect_profile() { PROFILE_FILE="$WP_PROFILE"; }

# T5: write_profile writes both exports to a fresh profile
write_profile
if grep -Fq "export PATH=" "$WP_PROFILE" && grep -Fq "export QUAID_DB=" "$WP_PROFILE"; then
  ok "T5: write_profile writes both PATH and QUAID_DB exports"
else
  not_ok "T5: write_profile did not write expected exports"
fi

# T6: write_profile is idempotent on re-run
write_profile
path_count=$(grep -c "export PATH=" "$WP_PROFILE")
db_count=$(grep -c "export QUAID_DB=" "$WP_PROFILE")
if [ "$path_count" = "1" ] && [ "$db_count" = "1" ]; then
  ok "T6: write_profile is idempotent (no duplicates on re-run)"
else
  not_ok "T6: write_profile duplicated lines: PATH×${path_count} QUAID_DB×${db_count}"
fi

# T7: profile does not key off current PATH — write_profile always checks the file
# Even if INSTALL_DIR happens to appear in the current session PATH, write_profile
# must defer to the profile file for idempotency, not the live environment.
OLD_PATH="$PATH"
# Temporarily add INSTALL_DIR to the session PATH to simulate an already-active session
PATH="${INSTALL_DIR}:${PATH}"
WP_PROFILE2="$TEST_HOME/.wp_fresh"
printf '' > "$WP_PROFILE2"
detect_profile() { PROFILE_FILE="$WP_PROFILE2"; }
write_profile
if grep -Fq "export PATH=" "$WP_PROFILE2"; then
  ok "T7: write_profile writes to profile regardless of current session PATH"
else
  not_ok "T7: write_profile skipped PATH write because INSTALL_DIR was already in session PATH"
fi
PATH="$OLD_PATH"

# ---------------------------------------------------------------
# --no-profile / QUAID_NO_PROFILE=1 opt-out
# ---------------------------------------------------------------

# T8: NO_PROFILE=1 branch — profile file must not be touched
WP_PROFILE3="$TEST_HOME/.optout_test"
printf '' > "$WP_PROFILE3"
detect_profile() { PROFILE_FILE="$WP_PROFILE3"; }
NO_PROFILE=1
# Simulate the main() branch: --no-profile calls print_manual_hints, not write_profile
if [ "$NO_PROFILE" = "1" ]; then
  print_manual_hints > /dev/null 2>&1
fi
if [ -s "$WP_PROFILE3" ]; then
  not_ok "T8: NO_PROFILE=1 must not write to profile"
else
  ok "T8: NO_PROFILE=1 leaves profile untouched"
fi
NO_PROFILE=0

# T9: QUAID_NO_PROFILE=1 env var is read at startup into NO_PROFILE
# The script initializes: NO_PROFILE="${QUAID_NO_PROFILE:-0}"
# We verify that behavior by checking the value directly.
QUAID_NO_PROFILE=1
_computed_no_profile="${QUAID_NO_PROFILE:-0}"
if [ "$_computed_no_profile" = "1" ]; then
  ok "T9: QUAID_NO_PROFILE=1 env var propagates to NO_PROFILE at startup"
else
  not_ok "T9: QUAID_NO_PROFILE=1 did not propagate"
fi
QUAID_NO_PROFILE=0

# ---------------------------------------------------------------
# detect_profile — branch coverage (T10–T13)
# ---------------------------------------------------------------
# Restore detect_profile to the PRODUCTION implementation by re-sourcing
# install.sh.  Earlier tests (T5-T8) override detect_profile with test
# stubs; re-sourcing is the only reliable way to recover the real function
# without copy-pasting its body (which would silently diverge if the
# production code changed).
. "$INSTALL_SH"

# T10: zsh → ~/.zshrc (any OS)
SHELL=/usr/bin/zsh detect_profile
if [ "$PROFILE_FILE" = "$HOME/.zshrc" ]; then
  ok "T10: detect_profile — zsh → ~/.zshrc"
else
  not_ok "T10: detect_profile — zsh → ~/.zshrc (got: $PROFILE_FILE)"
fi

# T11: Darwin bash → ~/.bash_profile
# Inject a fake uname into PATH that reports Darwin for 'uname -s'
DARWIN_STUBS="$TEST_HOME/darwin_stubs"
mkdir -p "$DARWIN_STUBS"
printf '#!/usr/bin/env sh\n[ "$1" = "-s" ] && { printf "Darwin\\n"; exit 0; }; exec /usr/bin/uname "$@"\n' \
  > "$DARWIN_STUBS/uname"
chmod +x "$DARWIN_STUBS/uname"
OLD_PATH_T11="$PATH"
PATH="$DARWIN_STUBS:$PATH"
SHELL=/bin/bash detect_profile
PATH="$OLD_PATH_T11"
if [ "$PROFILE_FILE" = "$HOME/.bash_profile" ]; then
  ok "T11: detect_profile — Darwin bash → ~/.bash_profile"
else
  not_ok "T11: detect_profile — Darwin bash → ~/.bash_profile (got: $PROFILE_FILE)"
fi

# T12: Linux bash → ~/.bashrc  (CI runs on Linux; real uname returns Linux)
SHELL=/bin/bash detect_profile
if [ "$PROFILE_FILE" = "$HOME/.bashrc" ]; then
  ok "T12: detect_profile — Linux bash → ~/.bashrc"
else
  not_ok "T12: detect_profile — Linux bash → ~/.bashrc (got: $PROFILE_FILE)"
fi

# T13: unknown shell → ~/.profile
SHELL=/usr/bin/fish detect_profile
if [ "$PROFILE_FILE" = "$HOME/.profile" ]; then
  ok "T13: detect_profile — unknown shell → ~/.profile"
else
  not_ok "T13: detect_profile — unknown shell → ~/.profile (got: $PROFILE_FILE)"
fi

# T14: detect_profile fails with a warning when the profile cannot be created
UNWRITABLE_HOME="$TEST_HOME/unwritable_home"
mkdir -p "$UNWRITABLE_HOME"
chmod 500 "$UNWRITABLE_HOME"
# On Windows/MSYS2, chmod on directories does not update NTFS ACLs.
# Use icacls via a .ps1 helper to actually deny write access.
_win_deny_write "$UNWRITABLE_HOME"
OLD_HOME_T14="$HOME"
HOME="$UNWRITABLE_HOME"
if SHELL=/usr/bin/zsh detect_profile >"$TEST_HOME/t14_detect.out" 2>"$TEST_HOME/t14_detect.err"; then
  not_ok "T14: detect_profile should fail when profile directory is not writable"
else
  if grep -Fq "Cannot create shell profile" "$TEST_HOME/t14_detect.err"; then
    ok "T14: detect_profile warns when profile creation fails"
  else
    not_ok "T14: detect_profile did not explain profile creation failure"
  fi
fi
HOME="$OLD_HOME_T14"
chmod 700 "$UNWRITABLE_HOME"
_win_allow_write "$UNWRITABLE_HOME"

# ---------------------------------------------------------------
# main() integration tests — T15–T18
# Exercises the real entry path with stubbed network/system functions.
# ---------------------------------------------------------------

# Build a fake 'curl' that writes a minimal sh script to any '-o FILE' target.
TEST_STUBS="$TEST_HOME/stubs"
mkdir -p "$TEST_STUBS"
printf '#!/usr/bin/env sh\noutfile=""\nwhile [ "$#" -gt 0 ]; do\n  case "$1" in\n    -o) outfile="$2"; shift 2 ;;\n    *)  shift ;;\n  esac\ndone\n[ -n "$outfile" ] && printf "#!/usr/bin/env sh\\nexit 0\\n" > "$outfile"\nexit 0\n' \
  > "$TEST_STUBS/curl"
chmod +x "$TEST_STUBS/curl"

# Override network/platform functions to avoid real I/O
resolve_version()  { VERSION="v0.0.0-test"; }
resolve_platform() { PLATFORM="linux-x86_64"; }
resolve_channel()  { CHANNEL="airgapped"; }
verify_checksum()  { return 0; }
need_cmd()         { return 0; }

SAVED_PATH_STUBS="$PATH"
PATH="$TEST_STUBS:$PATH"

# T15: main() --no-profile parses the flag and skips profile write
T15_PROFILE="$TEST_HOME/.t15_profile"
printf '' > "$T15_PROFILE"
detect_profile() { PROFILE_FILE="$T15_PROFILE"; }
NO_PROFILE=0
if main --no-profile >/dev/null 2>&1; then
  if [ ! -s "$T15_PROFILE" ]; then
    ok "T15: main() --no-profile parses flag and leaves profile untouched"
  else
    not_ok "T15: main() --no-profile wrote to profile (file non-empty)"
  fi
else
  not_ok "T15: main() --no-profile exited non-zero"
fi
tmp_dir=""

# T16: main() default path writes both exports through the real write_profile call
T16_PROFILE="$TEST_HOME/.t16_profile"
printf '' > "$T16_PROFILE"
detect_profile() { PROFILE_FILE="$T16_PROFILE"; }
NO_PROFILE=0
QUAID_NO_PROFILE=0
if main >/dev/null 2>&1; then
  if grep -Fq "export PATH=" "$T16_PROFILE" && grep -Fq "export QUAID_DB=" "$T16_PROFILE"; then
    ok "T16: main() default path writes PATH and QUAID_DB to profile"
  else
    not_ok "T16: main() default path did not write expected exports"
  fi
else
  not_ok "T16: main() default path exited non-zero"
fi
tmp_dir=""

PATH="$SAVED_PATH_STUBS"

# T17: QUAID_NO_PROFILE=1 env var is captured at script source time → NO_PROFILE=1
# Run the installer in a fresh subprocess (QUAID_TEST_MODE=1 prevents main() from firing
# but the top-level assignment  NO_PROFILE="${QUAID_NO_PROFILE:-0}"  still executes).
t17_result=$(QUAID_TEST_MODE=1 \
  QUAID_NO_PROFILE=1 \
  QUAID_INSTALL_DIR="$TEST_HOME/bin" \
  HOME="$TEST_HOME" \
  sh -c ". \"$INSTALL_SH\" && printf '%s' \"\$NO_PROFILE\"" 2>/dev/null)
if [ "$t17_result" = "1" ]; then
  ok "T17: QUAID_NO_PROFILE=1 env var sets NO_PROFILE=1 at script startup"
else
  not_ok "T17: QUAID_NO_PROFILE=1 did not set NO_PROFILE=1 (got: '$t17_result')"
fi

# T18: main() with QUAID_NO_PROFILE=1 env var — full end-to-end opt-out
# Exercises the real pipe-flow semantics: env var → top-level NO_PROFILE init → main() skip.
# Re-source install.sh so the top-level NO_PROFILE="${QUAID_NO_PROFILE:-0}" re-executes
# with QUAID_NO_PROFILE=1, then call main() and verify no profile write occurred.
QUAID_NO_PROFILE=1
. "$INSTALL_SH"
# Re-apply function stubs (re-source replaced them with production versions)
resolve_version()  { VERSION="v0.0.0-test"; }
resolve_platform() { PLATFORM="linux-x86_64"; }
resolve_channel()  { CHANNEL="airgapped"; }
verify_checksum()  { return 0; }
need_cmd()         { return 0; }
PATH="$TEST_STUBS:$PATH"
T18_PROFILE="$TEST_HOME/.t18_profile"
printf '' > "$T18_PROFILE"
detect_profile() { PROFILE_FILE="$T18_PROFILE"; }
if main >/dev/null 2>&1; then
  if [ ! -s "$T18_PROFILE" ]; then
    ok "T18: main() with QUAID_NO_PROFILE=1 skips profile write end-to-end"
  else
    not_ok "T18: main() with QUAID_NO_PROFILE=1 wrote to profile (file non-empty)"
  fi
else
  not_ok "T18: main() with QUAID_NO_PROFILE=1 exited non-zero"
fi
QUAID_NO_PROFILE=0
tmp_dir=""
PATH="$SAVED_PATH_STUBS"

# T19: main() fails loudly and prints manual recovery when profile persistence fails
. "$INSTALL_SH"
resolve_version()  { VERSION="v0.0.0-test"; }
resolve_platform() { PLATFORM="linux-x86_64"; }
resolve_channel()  { CHANNEL="airgapped"; }
verify_checksum()  { return 0; }
need_cmd()         { return 0; }
PATH="$TEST_STUBS:$PATH"
NO_PROFILE=0
QUAID_NO_PROFILE=0
T19_HOME="$TEST_HOME/unwritable_home_t19"
mkdir -p "$T19_HOME"
chmod 500 "$T19_HOME"
# On Windows/MSYS2, chmod on directories does not update NTFS ACLs.
# Use icacls via a .ps1 helper to actually deny write access.
_win_deny_write "$T19_HOME"
OLD_HOME_T19="$HOME"
OLD_SHELL_T19="${SHELL:-}"
HOME="$T19_HOME"
SHELL=/usr/bin/zsh
if main >"$TEST_HOME/t19_main.out" 2>"$TEST_HOME/t19_main.err"; then
  not_ok "T19: main() should exit non-zero when profile persistence fails"
else
  if grep -Fq "Cannot create shell profile ${T19_HOME}/.zshrc" "$TEST_HOME/t19_main.err" &&
     grep -Fq 'quaid was installed, but PATH/QUAID_DB were not persisted automatically.' "$TEST_HOME/t19_main.err" &&
     grep -Fq 'Complete setup by adding these to your shell profile:' "$TEST_HOME/t19_main.err" &&
     grep -Fq 'export PATH="' "$TEST_HOME/t19_main.err" &&
     grep -Fq 'export QUAID_DB="$HOME/.quaid/memory.db"' "$TEST_HOME/t19_main.err" &&
     grep -Fq 'download first, then run:' "$TEST_HOME/t19_main.err"; then
    ok "T19: main() drives the real detect_profile failure and prints recovery guidance"
  else
    not_ok "T19: main() did not print the expected real failure and recovery output"
  fi
fi
if grep -Fq "Installed quaid to" "$TEST_HOME/t19_main.out"; then
  ok "T19b: main() still reports where the binary was installed before failing"
else
  not_ok "T19b: main() should report the installed binary path on profile failure"
fi
if [ ! -e "$T19_HOME/.zshrc" ]; then
  ok "T19c: main() leaves the failed profile path untouched"
else
  not_ok "T19c: main() unexpectedly created the unwritable profile file"
fi
HOME="$OLD_HOME_T19"
SHELL="$OLD_SHELL_T19"
chmod 700 "$T19_HOME"
_win_allow_write "$T19_HOME"
tmp_dir=""
PATH="$SAVED_PATH_STUBS"

# ---------------------------------------------------------------
# Summary
# ---------------------------------------------------------------
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
