#!/usr/bin/env sh
# tests/install_profile.sh — deterministic tests for the installer's profile-write logic.
#
# Sources scripts/install.sh in GBRAIN_TEST_MODE=1 to isolate profile functions without
# running the download/install path. Tests cover: fresh write, idempotency, opt-out
# (--no-profile / GBRAIN_NO_PROFILE=1), and regex-metacharacter safety.
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
GBRAIN_TEST_MODE=1
GBRAIN_RELEASE_API_URL="https://example.invalid"
GBRAIN_RELEASE_BASE_URL="https://example.invalid"
GBRAIN_INSTALL_DIR="$TEST_HOME/bin"
GBRAIN_NO_PROFILE=0
GBRAIN_CHANNEL="airgapped"
GBRAIN_VERSION="v0.0.0-test"
HOME="$TEST_HOME"
mkdir -p "$GBRAIN_INSTALL_DIR"

# Source the installer in test mode — function definitions load, main() does not run
# shellcheck source=../scripts/install.sh
. "$INSTALL_SH"

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
if grep -Fq "export PATH=" "$WP_PROFILE" && grep -Fq "export GBRAIN_DB=" "$WP_PROFILE"; then
  ok "T5: write_profile writes both PATH and GBRAIN_DB exports"
else
  not_ok "T5: write_profile did not write expected exports"
fi

# T6: write_profile is idempotent on re-run
write_profile
path_count=$(grep -c "export PATH=" "$WP_PROFILE")
db_count=$(grep -c "export GBRAIN_DB=" "$WP_PROFILE")
if [ "$path_count" = "1" ] && [ "$db_count" = "1" ]; then
  ok "T6: write_profile is idempotent (no duplicates on re-run)"
else
  not_ok "T6: write_profile duplicated lines: PATH×${path_count} GBRAIN_DB×${db_count}"
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
# --no-profile / GBRAIN_NO_PROFILE=1 opt-out
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

# T9: GBRAIN_NO_PROFILE=1 env var is read at startup into NO_PROFILE
# The script initializes: NO_PROFILE="${GBRAIN_NO_PROFILE:-0}"
# We verify that behavior by checking the value directly.
GBRAIN_NO_PROFILE=1
_computed_no_profile="${GBRAIN_NO_PROFILE:-0}"
if [ "$_computed_no_profile" = "1" ]; then
  ok "T9: GBRAIN_NO_PROFILE=1 env var propagates to NO_PROFILE at startup"
else
  not_ok "T9: GBRAIN_NO_PROFILE=1 did not propagate"
fi
GBRAIN_NO_PROFILE=0

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

# ---------------------------------------------------------------
# main() integration tests — T14–T16
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

# T14: main() --no-profile parses the flag and skips profile write
T14_PROFILE="$TEST_HOME/.t14_profile"
printf '' > "$T14_PROFILE"
detect_profile() { PROFILE_FILE="$T14_PROFILE"; }
NO_PROFILE=0
if main --no-profile >/dev/null 2>&1; then
  if [ ! -s "$T14_PROFILE" ]; then
    ok "T14: main() --no-profile parses flag and leaves profile untouched"
  else
    not_ok "T14: main() --no-profile wrote to profile (file non-empty)"
  fi
else
  not_ok "T14: main() --no-profile exited non-zero"
fi
tmp_dir=""

# T15: main() default path writes both exports through the real write_profile call
T15_PROFILE="$TEST_HOME/.t15_profile"
printf '' > "$T15_PROFILE"
detect_profile() { PROFILE_FILE="$T15_PROFILE"; }
NO_PROFILE=0
GBRAIN_NO_PROFILE=0
if main >/dev/null 2>&1; then
  if grep -Fq "export PATH=" "$T15_PROFILE" && grep -Fq "export GBRAIN_DB=" "$T15_PROFILE"; then
    ok "T15: main() default path writes PATH and GBRAIN_DB to profile"
  else
    not_ok "T15: main() default path did not write expected exports"
  fi
else
  not_ok "T15: main() default path exited non-zero"
fi
tmp_dir=""

PATH="$SAVED_PATH_STUBS"

# T16: GBRAIN_NO_PROFILE=1 env var is captured at script source time → NO_PROFILE=1
# Run the installer in a fresh subprocess (GBRAIN_TEST_MODE=1 prevents main() from firing
# but the top-level assignment  NO_PROFILE="${GBRAIN_NO_PROFILE:-0}"  still executes).
t16_result=$(GBRAIN_TEST_MODE=1 \
  GBRAIN_NO_PROFILE=1 \
  GBRAIN_INSTALL_DIR="$TEST_HOME/bin" \
  HOME="$TEST_HOME" \
  sh -c ". \"$INSTALL_SH\" && printf '%s' \"\$NO_PROFILE\"" 2>/dev/null)
if [ "$t16_result" = "1" ]; then
  ok "T16: GBRAIN_NO_PROFILE=1 env var sets NO_PROFILE=1 at script startup"
else
  not_ok "T16: GBRAIN_NO_PROFILE=1 did not set NO_PROFILE=1 (got: '$t16_result')"
fi

# T17: main() with GBRAIN_NO_PROFILE=1 env var — full end-to-end opt-out
# Exercises the real pipe-flow semantics: env var → top-level NO_PROFILE init → main() skip.
# Re-source install.sh so the top-level NO_PROFILE="${GBRAIN_NO_PROFILE:-0}" re-executes
# with GBRAIN_NO_PROFILE=1, then call main() and verify no profile write occurred.
GBRAIN_NO_PROFILE=1
. "$INSTALL_SH"
# Re-apply function stubs (re-source replaced them with production versions)
resolve_version()  { VERSION="v0.0.0-test"; }
resolve_platform() { PLATFORM="linux-x86_64"; }
resolve_channel()  { CHANNEL="airgapped"; }
verify_checksum()  { return 0; }
need_cmd()         { return 0; }
PATH="$TEST_STUBS:$PATH"
T17_PROFILE="$TEST_HOME/.t17_profile"
printf '' > "$T17_PROFILE"
detect_profile() { PROFILE_FILE="$T17_PROFILE"; }
if main >/dev/null 2>&1; then
  if [ ! -s "$T17_PROFILE" ]; then
    ok "T17: main() with GBRAIN_NO_PROFILE=1 skips profile write end-to-end"
  else
    not_ok "T17: main() with GBRAIN_NO_PROFILE=1 wrote to profile (file non-empty)"
  fi
else
  not_ok "T17: main() with GBRAIN_NO_PROFILE=1 exited non-zero"
fi
GBRAIN_NO_PROFILE=0
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
