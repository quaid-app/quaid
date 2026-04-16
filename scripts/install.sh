#!/usr/bin/env sh

set -eu

REPO="macro88/gigabrain"
API_URL="https://api.github.com/repos/${REPO}/releases/latest"
RELEASE_BASE="https://github.com/${REPO}/releases/download"
INSTALL_DIR="${GBRAIN_INSTALL_DIR:-$HOME/.local/bin}"

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

print_path_hint() {
  case ":${PATH:-}:" in
    *:"$INSTALL_DIR":*) ;;
    *)
      printf '%s\n' ""
      printf '%s\n' "Add this directory to your PATH to run gbrain from anywhere:"
      printf '  export PATH="%s:$PATH"\n' "$INSTALL_DIR"
      ;;
  esac
}

print_gbrain_db_tip() {
  printf '%s\n' ""
  printf '%s\n' "Tip: Set GBRAIN_DB in your shell profile to avoid passing --db on every command:"
  printf '%s\n' "  echo 'export GBRAIN_DB=\"\$HOME/brain.db\"' >> ~/.zshrc"
  printf '%s\n' "  echo 'export GBRAIN_DB=\"\$HOME/brain.db\"' >> ~/.bashrc"
}

need_cmd curl
need_cmd mktemp
need_cmd mkdir
need_cmd chmod
need_cmd mv

trap cleanup EXIT INT HUP TERM

resolve_platform
resolve_version

asset_name="gbrain-${PLATFORM}"
checksum_name="${asset_name}.sha256"
binary_url="${RELEASE_BASE}/${VERSION}/${asset_name}"
checksum_url="${RELEASE_BASE}/${VERSION}/${checksum_name}"

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gbrain-install.XXXXXX")"

printf '%s\n' "Installing gbrain ${VERSION} for ${PLATFORM}..."
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
print_path_hint
print_gbrain_db_tip
