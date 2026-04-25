#!/usr/bin/env sh
# tests/install_release_seam.sh — verify installer asset naming matches release workflow artifacts.

set -eu

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
TEST_ROOT="$SCRIPT_DIR/target/test-install-release-seam"
STUBS_DIR="$TEST_ROOT/stubs"

rm -rf "$TEST_ROOT"
mkdir -p "$STUBS_DIR"

cat > "$STUBS_DIR/uname" <<'EOF'
#!/usr/bin/env sh
case "${1:-}" in
  -s) printf '%s\n' "${GBRAIN_TEST_UNAME_S:?}" ;;
  -m) printf '%s\n' "${GBRAIN_TEST_UNAME_M:?}" ;;
  *) exec /usr/bin/uname "$@" ;;
esac
EOF
chmod +x "$STUBS_DIR/uname"

GBRAIN_TEST_MODE=1
GBRAIN_RELEASE_API_URL="https://example.invalid"
GBRAIN_RELEASE_BASE_URL="https://example.invalid"
GBRAIN_INSTALL_DIR="$TEST_ROOT/bin"
GBRAIN_CHANNEL="airgapped"
GBRAIN_VERSION="v0.0.0-test"
HOME="$TEST_ROOT/home"
mkdir -p "$GBRAIN_INSTALL_DIR" "$HOME"

# shellcheck source=../scripts/install.sh
. "$INSTALL_SH"

ORIGINAL_PATH="$PATH"

check_case() {
  label="$1"
  uname_s="$2"
  uname_m="$3"
  channel="$4"
  expected_asset="$5"

  PATH="$STUBS_DIR:$ORIGINAL_PATH"
  export GBRAIN_TEST_UNAME_S="$uname_s"
  export GBRAIN_TEST_UNAME_M="$uname_m"
  GBRAIN_CHANNEL="$channel"

  resolve_platform
  resolve_channel
  asset_name="gbrain-${PLATFORM}-${CHANNEL}"
  checksum_name="${asset_name}.sha256"

  if [ "$asset_name" = "$expected_asset" ]; then
    ok "${label}: installer resolves ${expected_asset}"
  else
    not_ok "${label}: installer resolved ${asset_name} (expected ${expected_asset})"
  fi

  if grep -Fxq "${expected_asset}" "$MANIFEST"; then
    ok "${label}: canonical manifest includes ${expected_asset}"
  else
    not_ok "${label}: canonical manifest is missing ${expected_asset}"
  fi

  if grep -Fq "artifact: ${expected_asset}" "$RELEASE_YML"; then
    ok "${label}: release workflow builds ${expected_asset}"
  else
    not_ok "${label}: release workflow is missing artifact ${expected_asset}"
  fi

  if grep -Fq ".github/release-assets.txt" "$RELEASE_YML"; then
    ok "${label}: release workflow reads the canonical manifest for ${checksum_name}"
  else
    not_ok "${label}: release workflow does not read the canonical manifest for ${checksum_name}"
  fi

  if grep -Fxq "${checksum_name}" "$MANIFEST"; then
    ok "${label}: canonical manifest includes ${checksum_name}"
  else
    not_ok "${label}: canonical manifest is missing ${checksum_name}"
  fi
}

printf '\nRunning install/release seam tests...\n\n'

check_case "T1 darwin x86_64 airgapped" Darwin x86_64 airgapped gbrain-darwin-x86_64-airgapped
check_case "T2 darwin x86_64 online" Darwin x86_64 online gbrain-darwin-x86_64-online
check_case "T3 darwin arm64 airgapped" Darwin arm64 airgapped gbrain-darwin-arm64-airgapped
check_case "T4 darwin arm64 online" Darwin arm64 online gbrain-darwin-arm64-online
check_case "T5 linux x86_64 airgapped" Linux x86_64 airgapped gbrain-linux-x86_64-airgapped
check_case "T6 linux x86_64 online" Linux x86_64 online gbrain-linux-x86_64-online
check_case "T7 linux aarch64 airgapped" Linux aarch64 airgapped gbrain-linux-aarch64-airgapped
check_case "T8 linux aarch64 online" Linux aarch64 online gbrain-linux-aarch64-online

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"

if [ "$FAIL" -gt 0 ]; then
  exit 1
fi

exit 0
