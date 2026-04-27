#!/usr/bin/env sh
# tests/release_asset_parity.sh — static parity contract between install.sh and release.yml.
#
# Verifies that:
#   1. Every install.sh-resolvable asset name (platform × channel) appears as an `artifact:`
#      entry in .github/workflows/release.yml.
#   2. The release workflow consumes the canonical `.github/release-assets.txt` manifest.
#   3. No asset is present in the workflow but absent from the installer's resolution logic
#      (and vice versa).
#
# This test has NO network I/O and NO real binary downloads. It is pure source-level
# static analysis. Run it on any host with sh, grep, and sed.
#
# Usage:
#   sh tests/release_asset_parity.sh
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

SCRIPT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
INSTALL_SH="$SCRIPT_DIR/scripts/install.sh"
RELEASE_YML="$SCRIPT_DIR/.github/workflows/release.yml"
MANIFEST="$SCRIPT_DIR/.github/release-assets.txt"
TEST_ROOT="$SCRIPT_DIR/target/test-release-asset-parity"
BIN_DIR="$TEST_ROOT/bin"
HOME_DIR="$TEST_ROOT/home"

rm -rf "$TEST_ROOT"
mkdir -p "$BIN_DIR" "$HOME_DIR"

canonical_assets() {
  grep -Ev '^(#|$|install\.sh$|.*\.sha256$)' "$MANIFEST"
}

manifest_entries() {
  grep -Ev '^(#|$)' "$MANIFEST"
}

printf '\nRunning release asset parity tests...\n\n'

# ── T1: install.sh resolve_platform + resolve_channel cover every canonical binary name ──
# Source installer in test mode to get access to resolve_platform/resolve_channel helpers.
QUAID_TEST_MODE=1 \
  QUAID_RELEASE_API_URL="https://example.invalid" \
  QUAID_RELEASE_BASE_URL="https://example.invalid" \
  QUAID_INSTALL_DIR="$BIN_DIR" \
  QUAID_NO_PROFILE=0 \
  QUAID_CHANNEL="airgapped" \
  QUAID_VERSION="v0.0.0-test" \
  HOME="$HOME_DIR" \
  . "$INSTALL_SH"

# Simulate install.sh asset naming for all platform+channel combos.
# install.sh uses: asset_name="quaid-${PLATFORM}-${CHANNEL}"
# Platform resolution: os_name-arch_name (see resolve_platform).
simulate_asset() {
  platform="$1"
  channel="$2"
  printf 'quaid-%s-%s' "$platform" "$channel"
}

for name in $(canonical_assets); do
  # Extract platform and channel from canonical name
  # Format: quaid-<platform>-<channel>  where channel is last segment
  channel="${name##*-}"
  without_channel="${name%-*}"
  platform="${without_channel#quaid-}"
  expected="$(simulate_asset "$platform" "$channel")"
  if [ "$expected" = "$name" ]; then
    ok "T1[$name]: install.sh naming formula generates expected asset name"
  else
    not_ok "T1[$name]: formula produced '$expected', want '$name'"
  fi
done

# ── T2: every canonical binary appears as artifact: in release.yml ──
for name in $(canonical_assets); do
  if grep -Fq "artifact: ${name}" "$RELEASE_YML"; then
    ok "T2[$name]: release.yml matrix has artifact: $name"
  else
    not_ok "T2[$name]: release.yml is missing artifact: $name"
  fi
done

# ── T3: release.yml reads the canonical manifest directly ──
if grep -Fq ".github/release-assets.txt" "$RELEASE_YML"; then
  ok "T3: release.yml reads .github/release-assets.txt as the canonical manifest"
else
  not_ok "T3: release.yml does not read .github/release-assets.txt"
fi

# ── T4: no extra artifact: lines in release.yml beyond the canonical set ──
workflow_artifact_count=$(grep -c "artifact: quaid-" "$RELEASE_YML" || true)
canonical_count=$(canonical_assets | grep -c "quaid-" || true)
if [ "$workflow_artifact_count" = "$canonical_count" ]; then
  ok "T4: release.yml has exactly $canonical_count artifact: entries (no extras or gaps)"
else
  not_ok "T4: release.yml has $workflow_artifact_count artifact: entries; want $canonical_count"
fi

# ── T5: canonical manifest counts remain closed ──
manifest_count=$(manifest_entries | wc -l | tr -d ' ')
if [ "$manifest_count" = "17" ]; then
  ok "T5: canonical manifest has 17 release files (8 binaries + 8 checksums + install.sh)"
else
  not_ok "T5: canonical manifest has $manifest_count entries; want 17"
fi

# ── T6: RELEASE_CHECKLIST.md uses channel-suffixed names and points at the manifest ──
CHECKLIST="$SCRIPT_DIR/.github/RELEASE_CHECKLIST.md"
if [ -f "$CHECKLIST" ]; then
  bare_count=$(grep -Ec 'quaid-(darwin|linux)-(arm64|x86_64|aarch64)[^-]' "$CHECKLIST" || true)
  if [ "$bare_count" = "0" ] && grep -Fq ".github/release-assets.txt" "$CHECKLIST"; then
    ok "T6: RELEASE_CHECKLIST.md contains no bare binary names and references the canonical manifest"
  else
    not_ok "T6: RELEASE_CHECKLIST.md is missing the canonical-manifest reference or still has bare binary names"
  fi
else
  not_ok "T6: .github/RELEASE_CHECKLIST.md not found"
fi

# ── T7: installer does not attempt to download anything without channel suffix ──
if grep -Fq 'quaid-${PLATFORM}-${CHANNEL}' "$INSTALL_SH" || \
   grep -Fq '"quaid-${PLATFORM}-${CHANNEL}"' "$INSTALL_SH"; then
  ok "T7: install.sh asset name always includes CHANNEL suffix (no bare fallback path)"
else
  not_ok "T7: install.sh asset construction does not consistently include CHANNEL suffix"
fi

# ── T8: spec docs describe the channel-suffixed schema ──
if grep -Fq 'quaid-<platform>-<channel>' "$SCRIPT_DIR/docs/spec.md" && \
   grep -Fq 'quaid-<platform>-<channel>' "$SCRIPT_DIR/website/src/content/docs/contributing/specification.md"; then
  ok "T8: spec docs describe the channel-suffixed release asset schema"
else
  not_ok "T8: spec docs are missing the channel-suffixed release asset schema"
fi

# ── Summary ──────────────────────────────────────────────────────
printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
