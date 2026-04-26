#!/usr/bin/env bash
# benchmarks/prep_datasets.sh
#
# Download and verify pinned benchmark datasets to benchmarks/datasets/.
# ALL configuration is read from benchmarks/datasets.lock (TOML).
# No URLs, hashes, or commit SHAs are hardcoded in this script.
#
# Usage:
#   ./benchmarks/prep_datasets.sh              # download all datasets
#   ./benchmarks/prep_datasets.sh --verify-only  # verify cached archives
#   ./benchmarks/prep_datasets.sh --compute-hashes  # print SHA-256 of archives
#   ./benchmarks/prep_datasets.sh nq           # download only NQ subset
#   ./benchmarks/prep_datasets.sh fiqa         # download only FiQA subset
#
# Prerequisites:
#   curl, unzip, sha256sum (Linux) or shasum -a 256 (macOS), git
#
# CI caching:
#   Cache key: hash of benchmarks/datasets.lock
#   Cache path: benchmarks/datasets/
#
# Environment:
#   DATASETS_DIR — override default benchmarks/datasets/ path
#   SKIP_HASH_CHECK=1 — skip SHA-256 verification (NOT recommended)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
DATASETS_DIR="${DATASETS_DIR:-${SCRIPT_DIR}/datasets}"
LOCK_FILE="${SCRIPT_DIR}/datasets.lock"

# ── Lockfile parser ──────────────────────────────────────────────────────────

# Read a value from datasets.lock given a TOML section and key.
# Handles both top-level [section] and nested [section.subsection].
# Usage: lockfile_get "beir.nq" "sha256"  →  prints the value (unquoted)
lockfile_get() {
    local section="$1"
    local key="$2"

    [[ -f "$LOCK_FILE" ]] || die "Lockfile not found: ${LOCK_FILE}"

    # Convert dotted section to TOML header pattern
    local header
    header="$(echo "$section" | sed 's/\./\\]\\.\\[/g; s/^/\\[/; s/$/\\]/')"
    # e.g. "beir.nq" → "\[beir\]\.\[nq\]" which matches [beir.nq]
    # but we actually need to match both [beir.nq] and indented [beir.nq] forms

    local in_section=false
    local result=""

    while IFS= read -r line; do
        # Strip leading/trailing whitespace
        local trimmed
        trimmed="$(echo "$line" | sed 's/^[[:space:]]*//' | sed 's/[[:space:]]*$//')"

        # Skip comments and blank lines
        [[ -z "$trimmed" || "$trimmed" == \#* ]] && continue

        # Check for section headers
        if [[ "$trimmed" =~ ^\[.*\]$ ]]; then
            # Normalize the section header: strip brackets, spaces, and quotes
            local found_section
            found_section="$(echo "$trimmed" | sed 's/^\[*//; s/\]*$//; s/[[:space:]]//g')"
            if [[ "$found_section" == "$section" ]]; then
                in_section=true
            else
                # If we were in the target section and hit a new one, stop
                if $in_section; then
                    break
                fi
            fi
            continue
        fi

        # If we're in the target section, look for the key
        if $in_section && [[ "$trimmed" =~ ^${key}[[:space:]]*= ]]; then
            result="$(echo "$trimmed" | sed "s/^${key}[[:space:]]*=[[:space:]]*//" | sed 's/^"//; s/".*$//' | sed "s/^'//; s/'.*$//")"
            # Strip inline comments: value  # comment
            result="$(echo "$result" | sed 's/[[:space:]]*#.*$//')"
            break
        fi
    done < "$LOCK_FILE"

    if [[ -z "$result" ]]; then
        die "Key '${key}' not found in section [${section}] of ${LOCK_FILE}"
    fi

    echo "$result"
}

# ── Helpers ──────────────────────────────────────────────────────────────────

log()  { echo "[prep_datasets] $*"; }
warn() { echo "[prep_datasets] WARNING: $*" >&2; }
die()  { echo "[prep_datasets] ERROR: $*" >&2; exit 1; }

sha256_of() {
    local file="$1"
    if command -v sha256sum &>/dev/null; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum &>/dev/null; then
        shasum -a 256 "$file" | awk '{print $1}'
    else
        die "Neither sha256sum nor shasum found. Install one to verify downloads."
    fi
}

verify_hash() {
    local file="$1"
    local expected="$2"
    local name="$3"

    if [[ "${SKIP_HASH_CHECK:-0}" == "1" ]]; then
        warn "SKIP_HASH_CHECK=1 — skipping SHA-256 verification for ${name}"
        return 0
    fi

    local actual
    actual="$(sha256_of "$file")"
    if [[ "$actual" != "$expected" ]]; then
        die "SHA-256 mismatch for ${name}:
  expected: ${expected}
  actual:   ${actual}
  file:     ${file}"
    fi
    log "  ✓ SHA-256 verified: ${name}"
}

download_if_missing() {
    local url="$1"
    local dest="$2"
    local name="$3"

    if [[ -f "$dest" ]]; then
        log "  Cached: ${name} → ${dest}"
        return 0
    fi

    log "  Downloading: ${name}"
    log "  URL: ${url}"
    mkdir -p "$(dirname "$dest")"
    curl -fsSL --retry 3 --retry-delay 5 -o "$dest" "$url" \
        || die "Download failed: ${url}"
    log "  Downloaded: ${name} ($(du -sh "$dest" | cut -f1))"
}

extract_if_missing() {
    local archive="$1"
    local dest_dir="$2"
    local name="$3"

    if [[ -d "$dest_dir" ]] && [[ -n "$(ls -A "$dest_dir" 2>/dev/null)" ]]; then
        log "  Already extracted: ${name}"
        return 0
    fi

    log "  Extracting: ${name}"
    mkdir -p "$dest_dir"
    unzip -q "$archive" -d "$dest_dir" \
        || die "Extraction failed: ${archive}"
    log "  Extracted: ${name}"
}

clone_or_update() {
    local repo="$1"
    local commit="$2"
    local dest_dir="$3"
    local name="$4"

    if [[ -d "${dest_dir}/.git" ]]; then
        local current_commit
        current_commit="$(git -C "$dest_dir" rev-parse HEAD 2>/dev/null || echo "unknown")"
        if [[ "$current_commit" == "$commit"* ]]; then
            log "  Pinned commit already checked out: ${name} @ ${commit:0:12}"
            return 0
        fi
        log "  Updating to pinned commit: ${name} @ ${commit:0:12}"
        git -C "$dest_dir" fetch --quiet origin
        git -C "$dest_dir" checkout --quiet "$commit"
    else
        log "  Cloning: ${name}"
        mkdir -p "$(dirname "$dest_dir")"
        git clone --quiet "https://github.com/${repo}.git" "$dest_dir" \
            || die "Git clone failed: ${repo}"
        git -C "$dest_dir" checkout --quiet "$commit" \
            || die "Checkout failed: ${commit} in ${repo}"
        log "  Cloned and checked out: ${name} @ ${commit:0:12}"
    fi
}

# ── Mode flags ────────────────────────────────────────────────────────────────

MODE="download"  # download | verify-only | compute-hashes
SUBSET=""

for arg in "$@"; do
    case "$arg" in
        --verify-only)    MODE="verify-only" ;;
        --compute-hashes) MODE="compute-hashes" ;;
        nq|fiqa|longmemeval|locomo|all) SUBSET="$arg" ;;
        *) warn "Unknown argument: $arg" ;;
    esac
done

[[ -z "$SUBSET" ]] && SUBSET="all"

# ── BEIR: NQ ─────────────────────────────────────────────────────────────────

download_beir_nq() {
    local url
    url="$(lockfile_get "beir.nq" "url")"
    local expected_hash
    expected_hash="$(lockfile_get "beir.nq" "sha256")"
    local archive="${DATASETS_DIR}/beir/nq.zip"
    local dest_dir="${DATASETS_DIR}/beir/nq"

    case "$MODE" in
        compute-hashes)
            [[ -f "$archive" ]] && echo "nq: $(sha256_of "$archive")" || echo "nq: NOT DOWNLOADED"
            return ;;
        verify-only)
            [[ -f "$archive" ]] && verify_hash "$archive" "$expected_hash" "BEIR/NQ" \
                || warn "NQ archive not found at ${archive}"
            return ;;
    esac

    download_if_missing "$url" "$archive" "BEIR/NQ"
    verify_hash "$archive" "$expected_hash" "BEIR/NQ"
    extract_if_missing "$archive" "$dest_dir" "BEIR/NQ"
}

# ── BEIR: FiQA ───────────────────────────────────────────────────────────────

download_beir_fiqa() {
    local url
    url="$(lockfile_get "beir.fiqa" "url")"
    local expected_hash
    expected_hash="$(lockfile_get "beir.fiqa" "sha256")"
    local archive="${DATASETS_DIR}/beir/fiqa.zip"
    local dest_dir="${DATASETS_DIR}/beir/fiqa"

    case "$MODE" in
        compute-hashes)
            [[ -f "$archive" ]] && echo "fiqa: $(sha256_of "$archive")" || echo "fiqa: NOT DOWNLOADED"
            return ;;
        verify-only)
            [[ -f "$archive" ]] && verify_hash "$archive" "$expected_hash" "BEIR/FiQA" \
                || warn "FiQA archive not found at ${archive}"
            return ;;
    esac

    download_if_missing "$url" "$archive" "BEIR/FiQA"
    verify_hash "$archive" "$expected_hash" "BEIR/FiQA"
    extract_if_missing "$archive" "$dest_dir" "BEIR/FiQA"
}

# ── LongMemEval ───────────────────────────────────────────────────────────────

download_longmemeval() {
    local repo
    repo="$(lockfile_get "longmemeval" "repo")"
    local commit
    commit="$(lockfile_get "longmemeval" "commit")"
    local dest_dir="${DATASETS_DIR}/longmemeval"

    case "$MODE" in
        compute-hashes)
            echo "longmemeval: git clone at ${dest_dir} (no archive hash)"
            return ;;
        verify-only)
            if [[ -d "${dest_dir}/.git" ]]; then
                local current
                current="$(git -C "$dest_dir" rev-parse HEAD)"
                [[ "$current" == "$commit"* ]] \
                    && log "  ✓ LongMemEval pinned commit: ${commit:0:12}" \
                    || warn "LongMemEval commit mismatch: have ${current:0:12}, want ${commit:0:12}"
            else
                warn "LongMemEval not found at ${dest_dir}"
            fi
            return ;;
    esac

    clone_or_update "$repo" "$commit" "$dest_dir" "LongMemEval"
}

# ── LoCoMo ───────────────────────────────────────────────────────────────────

download_locomo() {
    local repo
    repo="$(lockfile_get "locomo" "repo")"
    local commit
    commit="$(lockfile_get "locomo" "commit")"
    local dest_dir="${DATASETS_DIR}/locomo"

    case "$MODE" in
        compute-hashes)
            echo "locomo: git clone at ${dest_dir} (no archive hash)"
            return ;;
        verify-only)
            if [[ -d "${dest_dir}/.git" ]]; then
                local current
                current="$(git -C "$dest_dir" rev-parse HEAD)"
                [[ "$current" == "$commit"* ]] \
                    && log "  ✓ LoCoMo pinned commit: ${commit:0:12}" \
                    || warn "LoCoMo commit mismatch: have ${current:0:12}, want ${commit:0:12}"
            else
                warn "LoCoMo not found at ${dest_dir}"
            fi
            return ;;
    esac

    clone_or_update "$repo" "$commit" "$dest_dir" "LoCoMo"
}

# ── Main ─────────────────────────────────────────────────────────────────────

log "=== Quaid Benchmark Dataset Prep ==="
log "Mode:     ${MODE}"
log "Subset:   ${SUBSET}"
log "Output:   ${DATASETS_DIR}"
log "Lockfile: ${LOCK_FILE}"
echo ""

# Validate lockfile exists
[[ -f "$LOCK_FILE" ]] || die "Lockfile not found: ${LOCK_FILE}"

mkdir -p "${DATASETS_DIR}"

case "$SUBSET" in
    all)
        download_beir_nq
        download_beir_fiqa
        download_longmemeval
        download_locomo
        ;;
    nq)          download_beir_nq ;;
    fiqa)        download_beir_fiqa ;;
    longmemeval) download_longmemeval ;;
    locomo)      download_locomo ;;
esac

echo ""
log "=== Done ==="

if [[ "$MODE" == "download" ]]; then
    log "Datasets ready in: ${DATASETS_DIR}"
    log "Run benchmarks with: cargo test --test beir_eval -- --ignored"
fi
