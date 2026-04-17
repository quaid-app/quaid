#!/usr/bin/env sh

set -eu

REPO="macro88/gigabrain"
API_URL="${GBRAIN_RELEASE_API_URL:-https://api.github.com/repos/${REPO}/releases/latest}"
RELEASE_BASE="${GBRAIN_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download}"
INSTALL_DIR="${GBRAIN_INSTALL_DIR:-$HOME/.local/bin}"
NO_PROFILE="${GBRAIN_NO_PROFILE:-0}"

tmp_dir=""

cleanup() {
  if [ -n "${tmp_dir}" ] && [ -d "${tmp_dir}" ]; then
    rm -rf "${tmp_dir}"
  fi
}

fail() {
  printf '%s\n' "Error: $*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "Required command not found: $1"
}

resolve_platform() {
  os="$(uname -s 2>/dev/null || true)"
  arch="$(uname -m 2>/dev/null || true)"

  case "$os" in
    Darwin) os_name="darwin" ;;
    Linux) os_name="linux" ;;
    *) fail "Unsupported operating system: ${os:-unknown}" ;;
  esac

  case "$arch" in
    x86_64|amd64) arch_name="x86_64" ;;
    aarch64|arm64)
      if [ "$os_name" = "darwin" ]; then
        arch_name="arm64"
      else
        arch_name="aarch64"
      fi
      ;;
    *) fail "Unsupported architecture: ${arch:-unknown}" ;;
  esac

  PLATFORM="${os_name}-${arch_name}"
}

resolve_version() {
  if [ -n "${GBRAIN_VERSION:-}" ]; then
    VERSION="$GBRAIN_VERSION"
    return
  fi

  release_json="$(curl -fsSL "$API_URL")" || fail "Failed to query latest release from GitHub API"
  VERSION="$(printf '%s' "$release_json" | tr -d '\n' | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"

  [ -n "$VERSION" ] || fail "Failed to parse tag_name from GitHub API response"
}

resolve_channel() {
  case "${GBRAIN_CHANNEL:-airgapped}" in
    airgapped) CHANNEL="airgapped" ;;
    online) CHANNEL="online" ;;
    *) fail "Unsupported GBRAIN_CHANNEL: ${GBRAIN_CHANNEL}. Use airgapped or online." ;;
  esac
}

verify_checksum() {
  checksum_file="$1"
  asset_name="$2"

  case "$PLATFORM" in
    darwin-*)
      need_cmd shasum
      if ! (cd "$tmp_dir" && shasum -a 256 --check "$checksum_file") >/dev/null 2>&1; then
        return 1
      fi
      ;;
    linux-*)
      need_cmd sha256sum
      if ! (cd "$tmp_dir" && sha256sum --check "$checksum_file") >/dev/null 2>&1; then
        return 1
      fi
      ;;
    *)
      fail "Unsupported platform for checksum verification: $PLATFORM"
      ;;
  esac

  [ -f "$tmp_dir/$asset_name" ] || fail "Checksum verification did not preserve downloaded binary"
}

# --- Profile detection and writing (A.1–A.3) ---

detect_profile() {
  shell_name="$(basename "${SHELL:-/bin/sh}" 2>/dev/null || echo "sh")"
  case "$shell_name" in
    zsh)
      PROFILE_FILE="$HOME/.zshrc"
      ;;
    bash)
      case "$(uname -s 2>/dev/null || true)" in
        Darwin) PROFILE_FILE="$HOME/.bash_profile" ;;
        *)      PROFILE_FILE="$HOME/.bashrc" ;;
      esac
      ;;
    *)
      PROFILE_FILE="$HOME/.profile"
      ;;
  esac

  # Create the profile file if it does not exist
  if [ ! -f "$PROFILE_FILE" ]; then
    touch "$PROFILE_FILE" 2>/dev/null || true
  fi
}

write_profile_line() {
  profile="$1"
  marker_pattern="$2"
  line_to_append="$3"

  if [ ! -f "$profile" ]; then
    return
  fi

  if grep -q "$marker_pattern" "$profile" 2>/dev/null; then
    return
  fi

  printf '\n%s\n' "$line_to_append" >> "$profile"
  printf '  Added: %s → %s\n' "$line_to_append" "$profile"
}

write_profile() {
  detect_profile

  path_line="export PATH=\"${INSTALL_DIR}:\$PATH\""
  db_line="export GBRAIN_DB=\"\$HOME/brain.db\""

  wrote_something=0

  # Only write PATH if install dir is not already in PATH
  case ":${PATH:-}:" in
    *:"$INSTALL_DIR":*) ;;
    *)
      write_profile_line "$PROFILE_FILE" "$INSTALL_DIR" "$path_line"
      wrote_something=1
      ;;
  esac

  write_profile_line "$PROFILE_FILE" "GBRAIN_DB" "$db_line"
  wrote_something=1

  if [ "$wrote_something" = "1" ]; then
    printf '\n  Profile updated: %s\n' "$PROFILE_FILE"
    printf '  Run: source %s\n' "$PROFILE_FILE"
  fi
}

print_manual_hints() {
  printf '%s\n' ""
  printf '%s\n' "Complete setup by adding these to your shell profile:"
  case ":${PATH:-}:" in
    *:"$INSTALL_DIR":*) ;;
    *)
      printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
      ;;
  esac
  printf '  export GBRAIN_DB="$HOME/brain.db"\n'
}

print_sandboxed_hint() {
  printf '%s\n' ""
  printf '%s\n' "For sandboxed/agent environments: download first, then run:"
  printf '  curl -fsSL https://raw.githubusercontent.com/%s/main/scripts/install.sh \\\n' "$REPO"
  printf '    -o gbrain-install.sh && sh gbrain-install.sh\n'
}

# --- Argument parsing ---

for arg in "$@"; do
  case "$arg" in
    --no-profile) NO_PROFILE=1 ;;
  esac
done

need_cmd curl
need_cmd mkdir
need_cmd chmod
need_cmd mv
need_cmd mktemp
need_cmd grep

trap cleanup EXIT INT HUP TERM

resolve_platform
resolve_channel
resolve_version

case "$CHANNEL" in
  airgapped|online) asset_name="gbrain-${PLATFORM}-${CHANNEL}" ;;
esac
checksum_name="${asset_name}.sha256"
binary_url="${RELEASE_BASE}/${VERSION}/${asset_name}"
checksum_url="${RELEASE_BASE}/${VERSION}/${checksum_name}"

tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t gbrain-install)" || fail "Cannot create temporary directory with mktemp"

printf '%s\n' "Installing gbrain ${VERSION} for ${PLATFORM} (${CHANNEL})..."
curl -fsSL "$binary_url" -o "$tmp_dir/$asset_name" || fail "Failed to download ${binary_url}"
curl -fsSL "$checksum_url" -o "$tmp_dir/$checksum_name" || fail "Failed to download ${checksum_url}"

if ! verify_checksum "$checksum_name" "$asset_name"; then
  fail "SHA-256 verification failed for ${asset_name}"
fi

if [ -d "$INSTALL_DIR" ] && [ ! -w "$INSTALL_DIR" ]; then
  fail "Install directory is not writable: ${INSTALL_DIR}
  Either use the default (~/.local/bin) or re-run with appropriate privileges:
    GBRAIN_INSTALL_DIR=\"\$HOME/.local/bin\" curl -fsSL ... | sh
    sudo sh -c 'GBRAIN_INSTALL_DIR=/usr/local/bin sh' < install.sh"
fi

if ! mkdir -p "$INSTALL_DIR" 2>/dev/null; then
  fail "Cannot create install directory: ${INSTALL_DIR}
  Try a user-writable path or run with appropriate privileges:
    GBRAIN_INSTALL_DIR=\"\$HOME/.local/bin\" curl -fsSL ... | sh"
fi

install_path="${INSTALL_DIR}/gbrain"
mv "$tmp_dir/$asset_name" "$install_path" || fail "Cannot write to ${install_path} — is the directory writable?"
chmod +x "$install_path"

if ! "$install_path" version; then
  rm -f "$install_path"
  fail "Smoke test failed: ${install_path} version"
fi

printf '%s\n' ""
printf 'Installed gbrain to %s\n' "$install_path"

if [ "$NO_PROFILE" = "1" ]; then
  print_manual_hints
else
  write_profile
fi

print_sandboxed_hint
