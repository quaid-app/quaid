#!/usr/bin/env sh

set -eu

REPO="quaid-app/quaid"
API_URL="${QUAID_RELEASE_API_URL:-https://api.github.com/repos/${REPO}/releases/latest}"
RELEASE_BASE="${QUAID_RELEASE_BASE_URL:-https://github.com/${REPO}/releases/download}"
INSTALL_DIR="${QUAID_INSTALL_DIR:-$HOME/.local/bin}"
NO_PROFILE="${QUAID_NO_PROFILE:-0}"

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
  if [ -n "${QUAID_VERSION:-}" ]; then
    VERSION="$QUAID_VERSION"
    return
  fi

  release_json="$(curl -fsSL "$API_URL")" || fail "Failed to query latest release from GitHub API"
  VERSION="$(printf '%s' "$release_json" | tr -d '\n' | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')"

  [ -n "$VERSION" ] || fail "Failed to parse tag_name from GitHub API response"
}

resolve_channel() {
  case "${QUAID_CHANNEL:-airgapped}" in
    airgapped) CHANNEL="airgapped" ;;
    online) CHANNEL="online" ;;
    *) fail "Unsupported QUAID_CHANNEL: ${QUAID_CHANNEL}. Use airgapped or online." ;;
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

  profile_dir="$(dirname "$PROFILE_FILE")"

  # Create the profile file if it does not exist
  if [ ! -f "$PROFILE_FILE" ]; then
    if [ ! -d "$profile_dir" ]; then
      printf '%s\n' "Warning: Cannot create shell profile ${PROFILE_FILE}; ${profile_dir} is not writable." >&2
      return 1
    fi

    # Probe actual write access: `[ -w dir ]` is unreliable on platforms where
    # chmod does not enforce directory write-access (e.g. Windows NTFS via Git
    # Bash), so a real I/O probe is the only portable gate.
    _probe="${profile_dir}/.quaid_install_probe_$$"
    if ! printf '' > "$_probe" 2>/dev/null; then
      rm -f "$_probe" 2>/dev/null || true
      printf '%s\n' "Warning: Cannot create shell profile ${PROFILE_FILE}; ${profile_dir} is not writable." >&2
      return 1
    fi
    rm -f "$_probe" 2>/dev/null || true

    if ! touch "$PROFILE_FILE" 2>/dev/null; then
      printf '%s\n' "Warning: Cannot create shell profile ${PROFILE_FILE}; ${profile_dir} is not writable." >&2
      return 1
    fi
  fi

  if [ ! -w "$PROFILE_FILE" ]; then
    printf '%s\n' "Warning: Shell profile ${PROFILE_FILE} is not writable." >&2
    return 1
  fi

  return 0
}

write_profile_line() {
  profile="$1"
  line_to_append="$2"

  if [ ! -f "$profile" ]; then
    return 2
  fi

  if grep -Fq "$line_to_append" "$profile" 2>/dev/null; then
    return 1
  fi

  if ! printf '\n%s\n' "$line_to_append" >> "$profile" 2>/dev/null; then
    printf '%s\n' "Warning: Failed to append to shell profile ${profile}." >&2
    return 2
  fi

  printf '  Added to %s: %s\n' "$profile" "$line_to_append"
  return 0
}

write_profile() {
  if ! detect_profile; then
    return 1
  fi

  path_line="export PATH=\"${INSTALL_DIR}:\$PATH\""
  db_line="export QUAID_DB=\"\$HOME/.quaid/memory.db\""

  wrote_something=0

  if write_profile_line "$PROFILE_FILE" "$path_line"; then
    wrote_something=1
  else
    status=$?
    if [ "$status" -gt 1 ]; then
      return 1
    fi
  fi

  if write_profile_line "$PROFILE_FILE" "$db_line"; then
    wrote_something=1
  else
    status=$?
    if [ "$status" -gt 1 ]; then
      return 1
    fi
  fi

  if [ "$wrote_something" = "1" ]; then
    printf '\n  Profile updated: %s\n' "$PROFILE_FILE"
    printf '  Run: source %s\n' "$PROFILE_FILE"
  else
    printf '\n  Profile already configured: %s\n' "$PROFILE_FILE"
  fi

  return 0
}

print_manual_hints() {
  printf '%s\n' ""
  printf '%s\n' "Complete setup by adding these to your shell profile:"
  printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
  printf '  export QUAID_DB="$HOME/.quaid/memory.db"\n'
}

print_sandboxed_hint() {
  printf '%s\n' ""
  printf '%s\n' "For sandboxed/agent environments: download first, then run:"
  printf '  curl -fsSL https://raw.githubusercontent.com/%s/main/scripts/install.sh \\\n' "$REPO"
  printf '    -o quaid-install.sh && sh quaid-install.sh\n'
}

# --- Main execution ---

main() {
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
    airgapped|online) asset_name="quaid-${PLATFORM}-${CHANNEL}" ;;
  esac
  checksum_name="${asset_name}.sha256"
  binary_url="${RELEASE_BASE}/${VERSION}/${asset_name}"
  checksum_url="${RELEASE_BASE}/${VERSION}/${checksum_name}"

  tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t quaid-install)" || fail "Cannot create temporary directory with mktemp"

  printf '%s\n' "Installing quaid ${VERSION} for ${PLATFORM} (${CHANNEL})..."
  curl -fsSL "$binary_url" -o "$tmp_dir/$asset_name" || fail "Failed to download ${binary_url}"
  curl -fsSL "$checksum_url" -o "$tmp_dir/$checksum_name" || fail "Failed to download ${checksum_url}"

  if ! verify_checksum "$checksum_name" "$asset_name"; then
    fail "SHA-256 verification failed for ${asset_name}"
  fi

  if [ -d "$INSTALL_DIR" ] && [ ! -w "$INSTALL_DIR" ]; then
    fail "Install directory is not writable: ${INSTALL_DIR}
  Either use the default (~/.local/bin) or re-run with appropriate privileges:
    curl -fsSL ... | QUAID_INSTALL_DIR=\"\$HOME/.local/bin\" sh
    sudo sh -c 'QUAID_INSTALL_DIR=/usr/local/bin sh' < install.sh"
  fi

  if ! mkdir -p "$INSTALL_DIR" 2>/dev/null; then
    fail "Cannot create install directory: ${INSTALL_DIR}
  Try a user-writable path or run with appropriate privileges:
    curl -fsSL ... | QUAID_INSTALL_DIR=\"\$HOME/.local/bin\" sh"
  fi

  install_path="${INSTALL_DIR}/quaid"
  mv "$tmp_dir/$asset_name" "$install_path" || fail "Cannot write to ${install_path} — is the directory writable?"
  chmod +x "$install_path"

  if ! "$install_path" version; then
    rm -f "$install_path"
    fail "Smoke test failed: ${install_path} version"
  fi

  printf '%s\n' ""
  printf 'Installed quaid to %s\n' "$install_path"

  printf '%s\n' ""
  printf '%s\n' "── Rename notice ──────────────────────────────────────────────────────────────"
  printf '%s\n' "  If you previously used an older binary under a different name, this install"
  printf '%s\n' "  does NOT migrate anything automatically. You must:"
  printf '%s\n' "    1. Update MCP client configs (Claude Code, Cursor, etc.) to use 'quaid'"
  printf '%s\n' "       and the new memory_* tool names — clients will see zero tools until done."
  printf '%s\n' "    2. Replace old env vars in your shell profile with QUAID_DB, QUAID_MODEL,"
  printf '%s\n' "       QUAID_CHANNEL, QUAID_INSTALL_DIR, etc."
  printf '%s\n' "    3. Migrate your database: export with the old binary, then run:"
  printf '%s\n' "         quaid init ~/.quaid/memory.db && quaid import <backup-dir/>"
  printf '%s\n' "  See https://github.com/quaid-app/quaid for the full migration guide."
  printf '%s\n' "───────────────────────────────────────────────────────────────────────────────"
  printf '%s\n' ""

  if [ "$NO_PROFILE" = "1" ]; then
    print_manual_hints
  else
    if ! write_profile; then
      printf '%s\n' "" >&2
      printf '%s\n' "Warning: quaid was installed, but PATH/QUAID_DB were not persisted automatically." >&2
      print_manual_hints >&2
      print_sandboxed_hint >&2
      return 1
    fi
  fi

  print_sandboxed_hint
}

if [ "${QUAID_TEST_MODE:-0}" != "1" ]; then
  main "$@"
fi
