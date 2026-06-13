//! Small-language-model lifecycle: alias resolution, local cache layout,
//! and (under the `online-model` feature) downloading model files from
//! HuggingFace with per-file integrity verification. The cache stores a
//! `manifest.json` describing every fetched artifact's SHA-256 so the
//! runtime can fail closed when files are missing, partial, or corrupt.
//! Curated aliases (`phi-3.5-mini`, `gemma-3-1b`, `gemma-3-4b`) are
//! source-pinned against published digests; arbitrary `<org>/<model>`
//! repo ids fall back to header-supplied hashes when available.
//!
//! See also: `super::slm` for the SLM runner that consumes the cached
//! files, `super::extractor` for the worker that loads a model on first
//! inference, and `crate::commands::model` for the CLI surface that drives
//! pull / status / clean operations against this module.

#[cfg(feature = "online-model")]
use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File};
use std::io::Read;
#[cfg(feature = "online-model")]
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
use sha1::Sha1;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[cfg(feature = "online-model")]
const DEFAULT_HUGGINGFACE_BASE_URL: &str = "https://huggingface.co";
const MODEL_CACHE_ROOT_ENV: &str = "QUAID_MODEL_CACHE_DIR";
#[cfg(feature = "online-model")]
const HUGGINGFACE_BASE_URL_ENV: &str = "QUAID_HF_BASE_URL";
const STALE_CACHE_TTL_ENV: &str = "QUAID_STALE_MODEL_CACHE_TTL_SECS";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const DOWNLOAD_HEARTBEAT_FILE: &str = ".downloading";
const MANIFEST_VERSION: u32 = 1;
const DEFAULT_STALE_DOWNLOAD_TTL: Duration = Duration::from_secs(6 * 60 * 60);

const PHI_35_MINI_REVISION: &str = "2fe192450127e6a83f7441aef6e3ca586c338b77";
const GEMMA_3_1B_REVISION: &str = "dcc83ea841ab6100d6b47a070329e1ba4cf78752";
const GEMMA_3_4B_REVISION: &str = "093f9f388b31de276ce2de164bdc2081324b9767";

// Test-only curated alias. Uses mixed SHA-256 / git-blob-SHA1 pins that match the
// fixture content in `mock_files(false)` inside tests/model_lifecycle.rs.
// This exercises the source-pinned download path (install_source_pinned_model_into_dir)
// without network access to real HuggingFace repos.
#[cfg(any(test, feature = "test-harness"))]
const TEST_PINNED_ALIAS: &str = "test-pinned";
#[cfg(any(test, feature = "test-harness"))]
const TEST_PINNED_REVISION: &str = "test-revision-abc123";
#[cfg(any(test, feature = "test-harness"))]
const TEST_CURATED_FILES: &[SourcePinnedFile] = &[
    // config.json: `{"model_type":"phi3"}` (21 bytes) — pin via git-blob-SHA1
    SourcePinnedFile {
        path: "config.json",
        digest: PinnedDigest::GitBlobSha1("3156de0790e76b33247de6adc66765c4fc608b42"),
    },
    // model.safetensors: `tiny-test-weights` (17 bytes) — pin via SHA-256
    SourcePinnedFile {
        path: "model.safetensors",
        digest: PinnedDigest::Sha256(
            "7f5bb0fd7b2f570f5c9140e319c619275a0b4960f8c64facf1f59b39511e1d6f",
        ),
    },
    // tokenizer.json: `{"version":"1.0"}` (17 bytes) — pin via git-blob-SHA1
    SourcePinnedFile {
        path: "tokenizer.json",
        digest: PinnedDigest::GitBlobSha1("22ec89ab90889036bc14d5b1bb6109974ef95582"),
    },
];

#[cfg(feature = "online-model")]
const OPTIONAL_SUPPORT_FILES: &[&str] = &[
    "added_tokens.json",
    "chat_template.json",
    "generation_config.json",
    "preprocessor_config.json",
    "processor_config.json",
    "special_tokens_map.json",
    "tokenizer.model",
    "tokenizer_config.json",
];

/// Reports download progress to a caller-specific UI.
pub trait ProgressReporter {
    /// Called once after metadata is fetched with the total file count the
    /// download plans to fetch.
    fn planned(&mut self, _alias: &ResolvedModelAlias, _file_count: usize) {}

    /// Called when an existing cache passes verification and no network
    /// fetch is needed.
    fn cache_hit(&mut self, _cache_dir: &Path) {}

    /// Called when an individual file fetch begins, with the expected byte
    /// length when the server provided one.
    fn file_started(&mut self, _file_name: &str, _bytes_total: Option<u64>) {}

    /// Called periodically while a file streams, with the running byte
    /// count and the expected total when known.
    fn file_progress(
        &mut self,
        _file_name: &str,
        _downloaded_bytes: u64,
        _bytes_total: Option<u64>,
    ) {
    }

    /// Called after a file fetch completes and its SHA-256 has been verified.
    fn file_finished(&mut self, _file_name: &str, _actual_sha256: &str) {}
}

/// Human-readable stderr reporter for CLI commands.
#[derive(Debug, Default)]
pub struct ConsoleProgressReporter {
    started_at: Option<Instant>,
    current_file_started_at: Option<Instant>,
    last_progress_at: Option<Instant>,
    planned_files: usize,
    finished_files: usize,
}

impl ProgressReporter for ConsoleProgressReporter {
    fn planned(&mut self, alias: &ResolvedModelAlias, file_count: usize) {
        self.started_at = Some(Instant::now());
        self.planned_files = file_count;
        eprintln!(
            "Downloading model `{}` from {}{} ({} file(s))",
            alias.requested_alias,
            alias.repo_id,
            alias
                .revision
                .as_deref()
                .map_or_else(String::new, |revision| { format!(" @ {revision}") }),
            file_count
        );
    }

    fn cache_hit(&mut self, cache_dir: &Path) {
        eprintln!("Model cache already verified at {}", cache_dir.display());
    }

    fn file_started(&mut self, file_name: &str, bytes_total: Option<u64>) {
        self.current_file_started_at = Some(Instant::now());
        self.last_progress_at = None;
        match bytes_total {
            Some(bytes_total) => eprintln!("  -> {file_name} ({})", human_bytes(bytes_total)),
            None => eprintln!("  -> {file_name}"),
        }
    }

    fn file_progress(&mut self, file_name: &str, downloaded_bytes: u64, bytes_total: Option<u64>) {
        let now = Instant::now();
        if self
            .last_progress_at
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(1))
            && bytes_total != Some(downloaded_bytes)
        {
            return;
        }
        self.last_progress_at = Some(now);

        let elapsed = self
            .current_file_started_at
            .map(|started| now.duration_since(started))
            .unwrap_or_default();
        let speed = if elapsed.as_secs_f64() > 0.0 {
            downloaded_bytes as f64 / elapsed.as_secs_f64()
        } else {
            0.0
        };
        let eta = bytes_total.and_then(|total| {
            if speed > 0.0 && total > downloaded_bytes {
                Some(Duration::from_secs_f64(
                    (total - downloaded_bytes) as f64 / speed,
                ))
            } else {
                None
            }
        });

        match (bytes_total, eta) {
            (Some(total), Some(eta)) => eprintln!(
                "     {file_name}: {} / {} ({}/s, ~{} remaining)",
                human_bytes(downloaded_bytes),
                human_bytes(total),
                human_bytes(speed as u64),
                human_duration(eta)
            ),
            (Some(total), None) => eprintln!(
                "     {file_name}: {} / {}",
                human_bytes(downloaded_bytes),
                human_bytes(total)
            ),
            (None, _) => eprintln!("     {file_name}: {}", human_bytes(downloaded_bytes)),
        }
    }

    fn file_finished(&mut self, file_name: &str, actual_sha256: &str) {
        self.finished_files += 1;
        let elapsed = self
            .started_at
            .map(|started| human_duration(started.elapsed()))
            .unwrap_or_else(|| "unknown time".to_owned());
        let digest_prefix = actual_sha256.get(..12).unwrap_or(actual_sha256);
        eprintln!(
            "  ok {file_name} (SHA-256: {digest_prefix}..., {}/{}, {elapsed})",
            self.finished_files, self.planned_files
        );
        if self.planned_files > 0 && self.finished_files >= self.planned_files {
            eprintln!(
                "Download complete: {} file(s) verified in {elapsed}",
                self.finished_files
            );
        }
    }
}

/// Silent reporter for tests and non-interactive callers.
#[derive(Debug, Default)]
pub struct NoopProgressReporter;

impl ProgressReporter for NoopProgressReporter {}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn human_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let remainder = seconds % 60;
    if minutes < 60 {
        return format!("{minutes}m{remainder:02}s");
    }
    let hours = minutes / 60;
    let minutes = minutes % 60;
    format!("{hours}h{minutes:02}m")
}

/// Result of resolving a user-supplied alias to the concrete HuggingFace
/// repo, revision, and cache key that the lifecycle code operates on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelAlias {
    /// Alias exactly as the user supplied it, after trimming.
    pub requested_alias: String,
    /// Filesystem-safe directory name used under the model cache root.
    pub cache_key: String,
    /// HuggingFace repo id (`<org>/<model>`).
    pub repo_id: String,
    /// Pinned revision (commit SHA or tag) when the alias has one.
    pub revision: Option<String>,
}

/// Snapshot of a model's local cache directory, including whether the
/// manifest verified cleanly and whether every file was fetched against a
/// source pin (versus a header-supplied hash).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedModelStatus {
    /// Resolved alias the status was computed for.
    pub alias: ResolvedModelAlias,
    /// Filesystem path of the cache directory.
    pub cache_dir: PathBuf,
    /// `true` when the cache directory exists on disk.
    pub is_cached: bool,
    /// `true` when the manifest could be read and every recorded file's
    /// SHA-256 still matches the on-disk content.
    pub verified: bool,
    /// `true` when every file in the manifest was verified against a
    /// source-pinned digest, not a server-supplied hash.
    pub source_pinned: bool,
}

/// High-level family for a cache entry discovered under the shared model
/// cache root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCacheFamily {
    /// Chat/extraction SLM cache managed by `quaid model pull`.
    Extraction,
    /// Embedding model cache managed by semantic indexing/query paths.
    Embedding,
}

impl ModelCacheFamily {
    /// Stable lowercase label for CLI output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extraction => "extraction",
            Self::Embedding => "embedding",
        }
    }
}

impl fmt::Display for ModelCacheFamily {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Validation state for a discovered model cache path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCacheState {
    /// A complete cache passed the requested validation level.
    Complete,
    /// The expected cache path is absent.
    Missing,
    /// The cache path exists but is missing required files or metadata.
    Incomplete,
    /// The cache path exists but validation detected corrupt or mismatched content.
    Corrupted,
    /// A temporary download path is older than the stale-download threshold.
    StaleTemporary,
    /// A temporary download path still has a fresh heartbeat or mtime.
    ActiveTemporary,
}

impl ModelCacheState {
    /// Stable lowercase label for CLI output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Missing => "missing",
            Self::Incomplete => "incomplete",
            Self::Corrupted => "corrupted",
            Self::StaleTemporary => "stale-temp",
            Self::ActiveTemporary => "active-temp",
        }
    }
}

impl fmt::Display for ModelCacheState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Snapshot of one extraction or embedding cache path discovered for
/// operator-facing status and cleanup commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCacheEntry {
    /// Cache family.
    pub family: ModelCacheFamily,
    /// User-facing alias when the path can be mapped to one.
    pub alias: Option<String>,
    /// Sanitized cache key under the shared cache root.
    pub cache_key: String,
    /// Filesystem path represented by the entry.
    pub path: PathBuf,
    /// Validation state for the path.
    pub state: ModelCacheState,
    /// Human-readable detail about the state.
    pub reason: String,
    /// Number of regular files under the path.
    pub file_count: usize,
    /// Total bytes occupied by regular files under the path.
    pub size_bytes: u64,
    /// Latest observed mtime, represented as seconds since the Unix epoch.
    pub modified_unix: Option<u64>,
    /// `true` when cleanup may remove this entry without deleting a verified
    /// complete cache.
    pub cleanup_eligible: bool,
    /// `true` when the entry represents a complete cache that should only be
    /// removed when an alias-specific forced clean explicitly targets it.
    pub complete_cache: bool,
}

/// Result for a single cleanup removal attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCacheRemoval {
    /// Filesystem path targeted by the removal attempt.
    pub path: PathBuf,
    /// Bytes that were counted for the entry before removal.
    pub size_bytes: u64,
    /// Error message when removal failed.
    pub error: Option<String>,
}

/// Summary returned after removing one or more cache entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCacheCleanReport {
    /// Successful removals.
    pub removed: Vec<ModelCacheRemoval>,
    /// Failed removals.
    pub failed: Vec<ModelCacheRemoval>,
    /// Sum of `size_bytes` for successful removals.
    pub bytes_freed: u64,
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PinnedDigest {
    Sha256(&'static str),
    GitBlobSha1(&'static str),
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourcePinnedFile {
    path: &'static str,
    digest: PinnedDigest,
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
const PHI_35_MINI_FILES: &[SourcePinnedFile] = &[
    SourcePinnedFile {
        path: "added_tokens.json",
        digest: PinnedDigest::GitBlobSha1("178968dec606c790aa335e9142f6afec37288470"),
    },
    SourcePinnedFile {
        path: "config.json",
        digest: PinnedDigest::GitBlobSha1("62d1ca8d4b3c8ab21ac9a30775e1a224be66ef58"),
    },
    SourcePinnedFile {
        path: "generation_config.json",
        digest: PinnedDigest::GitBlobSha1("93dfd6366eab0bbae1613c5deab3a43bc989e5f8"),
    },
    SourcePinnedFile {
        path: "model.safetensors.index.json",
        digest: PinnedDigest::GitBlobSha1("9d8ea7b588c536f06b274d405d1c3281cb722602"),
    },
    SourcePinnedFile {
        path: "model-00001-of-00002.safetensors",
        digest: PinnedDigest::Sha256(
            "c5214cdb995ed3dd716add8d9efbfe016b76bb2f1c4c1e6c1c6a95497d7a8837",
        ),
    },
    SourcePinnedFile {
        path: "model-00002-of-00002.safetensors",
        digest: PinnedDigest::Sha256(
            "41246eed2b75b66526339c5d32d6f7acdefe0bd24180f97c74303f4656877344",
        ),
    },
    SourcePinnedFile {
        path: "special_tokens_map.json",
        digest: PinnedDigest::GitBlobSha1("badfd2a349071a6b3ae2681838e51695781f60e1"),
    },
    SourcePinnedFile {
        path: "tokenizer.json",
        digest: PinnedDigest::GitBlobSha1("e213d846868b09aefa5225e6026d91b41e19e7ff"),
    },
    SourcePinnedFile {
        path: "tokenizer.model",
        digest: PinnedDigest::Sha256(
            "9e556afd44213b6bd1be2b850ebbbd98f5481437a8021afaf58ee7fb1818d347",
        ),
    },
    SourcePinnedFile {
        path: "tokenizer_config.json",
        digest: PinnedDigest::GitBlobSha1("7280c871b0d18a2a03000fb8d638983793aa33e1"),
    },
];

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
const GEMMA_3_1B_FILES: &[SourcePinnedFile] = &[
    SourcePinnedFile {
        path: "added_tokens.json",
        digest: PinnedDigest::GitBlobSha1("e17bde03d42feda32d1abfca6d3b598b9a020df7"),
    },
    SourcePinnedFile {
        path: "config.json",
        digest: PinnedDigest::GitBlobSha1("06ab0678bc32d4f1474f31b35914512b1f9edaf7"),
    },
    SourcePinnedFile {
        path: "generation_config.json",
        digest: PinnedDigest::GitBlobSha1("37a4c871d263a349f50e4a313db3e72164950702"),
    },
    SourcePinnedFile {
        path: "model.safetensors",
        digest: PinnedDigest::Sha256(
            "3d4ef8d71c14db7e448a09ebe891cfb6bf32c57a9b44499ae0d1c098e48516b6",
        ),
    },
    SourcePinnedFile {
        path: "special_tokens_map.json",
        digest: PinnedDigest::GitBlobSha1("1a6193244714d3d78be48666cb02cdbfac62ad86"),
    },
    SourcePinnedFile {
        path: "tokenizer.json",
        digest: PinnedDigest::Sha256(
            "4667f2089529e8e7657cfb6d1c19910ae71ff5f28aa7ab2ff2763330affad795",
        ),
    },
    SourcePinnedFile {
        path: "tokenizer.model",
        digest: PinnedDigest::Sha256(
            "1299c11d7cf632ef3b4e11937501358ada021bbdf7c47638d13c0ee982f2e79c",
        ),
    },
    SourcePinnedFile {
        path: "tokenizer_config.json",
        digest: PinnedDigest::GitBlobSha1("7bdd14f0eaec30c8d2c56bc9d543587676e19c0f"),
    },
];

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
const GEMMA_3_4B_FILES: &[SourcePinnedFile] = &[
    SourcePinnedFile {
        path: "added_tokens.json",
        digest: PinnedDigest::GitBlobSha1("e17bde03d42feda32d1abfca6d3b598b9a020df7"),
    },
    SourcePinnedFile {
        path: "chat_template.json",
        digest: PinnedDigest::GitBlobSha1("719b0cd0d7a373a400b0c119ee0e051f41ea88d9"),
    },
    SourcePinnedFile {
        path: "config.json",
        digest: PinnedDigest::GitBlobSha1("fcdc8a6cd637c7ae460065204e7fd0d8c00a38fc"),
    },
    SourcePinnedFile {
        path: "generation_config.json",
        digest: PinnedDigest::GitBlobSha1("37a4c871d263a349f50e4a313db3e72164950702"),
    },
    SourcePinnedFile {
        path: "model.safetensors.index.json",
        digest: PinnedDigest::GitBlobSha1("4b95241f208f06d324d17c9675568ec58dafd9fb"),
    },
    SourcePinnedFile {
        path: "model-00001-of-00002.safetensors",
        digest: PinnedDigest::Sha256(
            "eb5fd5e97ddd07b56778733e9653c07312529cb00980a318fc3e1c4e3b5a8f1f",
        ),
    },
    SourcePinnedFile {
        path: "model-00002-of-00002.safetensors",
        digest: PinnedDigest::Sha256(
            "fdde0e5aa5ced0fa203b3d50f4ab78168b7e3a3e08c6349f5cc9326666e1bb13",
        ),
    },
    SourcePinnedFile {
        path: "preprocessor_config.json",
        digest: PinnedDigest::GitBlobSha1("b1e00fc184f61b698181821169c6374cd5813e5c"),
    },
    SourcePinnedFile {
        path: "processor_config.json",
        digest: PinnedDigest::GitBlobSha1("453c7966d4b5d0b4a317c585989f64c58c2a6bf0"),
    },
    SourcePinnedFile {
        path: "special_tokens_map.json",
        digest: PinnedDigest::GitBlobSha1("1a6193244714d3d78be48666cb02cdbfac62ad86"),
    },
    SourcePinnedFile {
        path: "tokenizer.json",
        digest: PinnedDigest::Sha256(
            "4667f2089529e8e7657cfb6d1c19910ae71ff5f28aa7ab2ff2763330affad795",
        ),
    },
    SourcePinnedFile {
        path: "tokenizer.model",
        digest: PinnedDigest::Sha256(
            "1299c11d7cf632ef3b4e11937501358ada021bbdf7c47638d13c0ee982f2e79c",
        ),
    },
    SourcePinnedFile {
        path: "tokenizer_config.json",
        digest: PinnedDigest::GitBlobSha1("7bdd14f0eaec30c8d2c56bc9d543587676e19c0f"),
    },
];

/// Errors returned from alias resolution, cache verification, and (under
/// the `online-model` feature) model download.
#[derive(Debug, Error)]
pub enum ModelLifecycleError {
    /// The crate was built without the `online-model` feature, so network
    /// downloads are not compiled in.
    #[error("model download support requires the online-model build")]
    DownloadsUnsupported,

    /// The supplied alias was empty or did not parse as a curated alias or
    /// a valid `<org>/<model>` repo id.
    #[error("invalid model alias `{alias}`: {message}")]
    InvalidAlias {
        /// Alias the caller supplied.
        alias: String,
        /// Human-readable reason the alias was rejected.
        message: String,
    },

    /// A repo id that the alias resolved to failed structural validation.
    #[error("invalid model repo `{repo_id}`: {message}")]
    InvalidRepo {
        /// Repo id under validation.
        repo_id: String,
        /// Human-readable reason the repo id was rejected.
        message: String,
    },

    /// Neither `QUAID_MODEL_CACHE_DIR` nor a home directory was available
    /// to host the model cache.
    #[error("could not resolve a model cache directory")]
    CacheRootUnavailable,

    /// A network or filesystem step of the download pipeline failed.
    #[error("download failed for `{alias}`: {message}")]
    Download {
        /// Alias the failed download was for.
        alias: String,
        /// Human-readable explanation of the failure.
        message: String,
    },

    /// The on-disk cache is present but its manifest or file digests do
    /// not match expectations.
    #[error("model cache at {cache_dir} is invalid: {message}")]
    CacheInvalid {
        /// Path of the offending cache directory.
        cache_dir: String,
        /// Human-readable explanation of the mismatch.
        message: String,
    },

    /// HuggingFace metadata for the model could not be fetched or parsed,
    /// or required artifacts were missing from the sibling list.
    #[error("model metadata for `{alias}` is incomplete: {message}")]
    Metadata {
        /// Alias the metadata fetch was for.
        alias: String,
        /// Human-readable explanation of the metadata problem.
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheManifest {
    #[serde(default)]
    manifest_version: Option<u32>,
    requested_alias: String,
    repo_id: String,
    revision: Option<String>,
    #[serde(default)]
    created_at_unix: Option<u64>,
    files: Vec<CachedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CachedFile {
    path: String,
    sha256: String,
    #[serde(default)]
    size_bytes: Option<u64>,
    #[serde(default)]
    modified_unix: Option<u64>,
    verified_from_source: bool,
}

#[cfg(feature = "online-model")]
#[derive(Debug, Deserialize)]
struct ModelMetadata {
    siblings: Vec<ModelSibling>,
}

#[cfg(feature = "online-model")]
#[derive(Debug, Deserialize)]
struct ModelSibling {
    rfilename: String,
}

#[cfg(feature = "online-model")]
#[derive(Debug)]
struct DownloadedArtifact {
    relative_path: String,
    sha256: String,
    verified_from_source: bool,
}

#[cfg(feature = "online-model")]
#[derive(Debug)]
struct TempDirCleanupGuard {
    path: PathBuf,
    armed: bool,
}

#[cfg(feature = "online-model")]
impl TempDirCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    fn cleanup_now(&mut self) -> Result<(), String> {
        if self.path.exists() {
            fs::remove_dir_all(&self.path)
                .map_err(|error| format!("remove {}: {error}", self.path.display()))?;
        }
        self.disarm();
        Ok(())
    }
}

#[cfg(feature = "online-model")]
impl Drop for TempDirCleanupGuard {
    fn drop(&mut self) {
        if !self.armed || !self.path.exists() {
            return;
        }
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "Warning: failed to remove temporary model cache at {}: {error}. Run `quaid model clean --all --force` after the download exits.",
                self.path.display()
            );
        }
    }
}

/// Map a user-facing alias (curated short name or raw `<org>/<model>` repo
/// id) into a [`ResolvedModelAlias`] with a stable cache key and optional
/// pinned revision; rejects malformed inputs before any I/O happens.
pub fn resolve_model_alias(alias: &str) -> Result<ResolvedModelAlias, ModelLifecycleError> {
    let trimmed = alias.trim();
    if trimmed.is_empty() {
        return Err(ModelLifecycleError::InvalidAlias {
            alias: alias.to_owned(),
            message: "expected a non-empty alias or <org>/<model> repo id".to_owned(),
        });
    }

    let normalized = trimmed.to_ascii_lowercase();
    let (repo_id, revision) = match normalized.as_str() {
        "phi-3.5-mini" => (
            "microsoft/Phi-3.5-mini-instruct".to_owned(),
            Some(PHI_35_MINI_REVISION.to_owned()),
        ),
        "gemma-3-1b" => (
            "google/gemma-3-1b-it".to_owned(),
            Some(GEMMA_3_1B_REVISION.to_owned()),
        ),
        "gemma-3-4b" => (
            "google/gemma-3-4b-it".to_owned(),
            Some(GEMMA_3_4B_REVISION.to_owned()),
        ),
        #[cfg(any(test, feature = "test-harness"))]
        TEST_PINNED_ALIAS => (
            "test-org/test-pinned-model".to_owned(),
            Some(TEST_PINNED_REVISION.to_owned()),
        ),
        _ => {
            validate_repo_id(trimmed).map_err(|message| ModelLifecycleError::InvalidAlias {
                alias: trimmed.to_owned(),
                message,
            })?;
            (trimmed.to_owned(), None)
        }
    };

    validate_repo_id(&repo_id).map_err(|message| ModelLifecycleError::InvalidRepo {
        repo_id: repo_id.clone(),
        message,
    })?;

    Ok(ResolvedModelAlias {
        requested_alias: trimmed.to_owned(),
        cache_key: sanitize_cache_key(trimmed),
        repo_id,
        revision,
    })
}

/// Compute the cache directory path for an alias without touching the
/// network or the filesystem; useful for callers that want to know
/// "where would this go?" before deciding to pull.
pub fn cache_dir_for_alias(alias: &str) -> Result<PathBuf, ModelLifecycleError> {
    let alias = resolve_model_alias(alias)?;
    cache_dir_for_resolved_alias(&alias)
}

/// Inspect the local cache for an alias and return whether it is present,
/// whether the manifest verifies, and whether every artifact was fetched
/// against a source-pinned digest.
pub fn cached_model_status(alias: &str) -> Result<CachedModelStatus, ModelLifecycleError> {
    let alias = resolve_model_alias(alias)?;
    let cache_dir = cache_dir_for_resolved_alias(&alias)?;
    let (is_cached, verified, source_pinned) = if cache_dir.is_dir() {
        match validated_manifest(&cache_dir, &alias) {
            Ok(manifest) => (
                true,
                true,
                manifest.files.iter().all(|file| file.verified_from_source),
            ),
            Err(_) => (true, false, false),
        }
    } else {
        (false, false, false)
    };

    Ok(CachedModelStatus {
        alias,
        cache_dir,
        is_cached,
        verified,
        source_pinned,
    })
}

/// Inspect extraction and embedding cache entries under the shared model
/// cache root.
///
/// When `alias` is `Some`, the result includes missing expected paths for
/// matching families. When `verify_hashes` is `true`, complete caches must
/// pass full digest verification instead of fast metadata validation.
pub fn inspect_model_caches(
    alias: Option<&str>,
    verify_hashes: bool,
) -> Result<Vec<ModelCacheEntry>, ModelLifecycleError> {
    let cache_root = cache_root_dir()?;
    let mode = if verify_hashes {
        ManifestValidation::Full
    } else {
        ManifestValidation::Fast
    };
    let mut entries = Vec::new();

    if let Some(selector) = alias {
        let mut matched = false;
        if let Ok(resolved) = resolve_model_alias(selector) {
            matched = true;
            inspect_extraction_alias(&mut entries, &cache_root, &resolved, mode)?;
        }

        #[cfg(feature = "online-model")]
        if let Some(model) = crate::core::inference::resolve_known_embedding_model(selector) {
            matched = true;
            inspect_embedding_model(&mut entries, &cache_root, &model, true, verify_hashes);
        }

        if !matched {
            resolve_model_alias(selector)?;
        }
    } else {
        inspect_extraction_cache_root(&mut entries, &cache_root, mode)?;

        #[cfg(feature = "online-model")]
        for model in crate::core::inference::known_embedding_models() {
            inspect_embedding_model(&mut entries, &cache_root, &model, false, verify_hashes);
        }
    }

    entries.sort_by(|left, right| {
        left.family
            .as_str()
            .cmp(right.family.as_str())
            .then_with(|| left.cache_key.cmp(&right.cache_key))
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(entries)
}

/// Remove cache entries that were selected by an operator-facing cleanup
/// command.
///
/// The removal is constrained to the resolved model cache root. Symlinks are
/// removed as symlinks; directories are removed recursively.
pub fn remove_model_cache_entries(entries: &[ModelCacheEntry]) -> ModelCacheCleanReport {
    let mut report = ModelCacheCleanReport::default();
    let Ok(cache_root) = cache_root_dir() else {
        for entry in entries {
            report.failed.push(ModelCacheRemoval {
                path: entry.path.clone(),
                size_bytes: entry.size_bytes,
                error: Some("could not resolve model cache root".to_owned()),
            });
        }
        return report;
    };

    for entry in entries {
        if !entry.cleanup_eligible && !entry.complete_cache {
            report.failed.push(ModelCacheRemoval {
                path: entry.path.clone(),
                size_bytes: entry.size_bytes,
                error: Some("entry is not eligible for cleanup".to_owned()),
            });
            continue;
        }
        if !path_is_within_cache_root(&cache_root, &entry.path) {
            report.failed.push(ModelCacheRemoval {
                path: entry.path.clone(),
                size_bytes: entry.size_bytes,
                error: Some("refusing to remove a path outside the model cache root".to_owned()),
            });
            continue;
        }

        match remove_cache_path(&entry.path) {
            Ok(()) => {
                report.bytes_freed = report.bytes_freed.saturating_add(entry.size_bytes);
                report.removed.push(ModelCacheRemoval {
                    path: entry.path.clone(),
                    size_bytes: entry.size_bytes,
                    error: None,
                });
            }
            Err(error) => report.failed.push(ModelCacheRemoval {
                path: entry.path.clone(),
                size_bytes: entry.size_bytes,
                error: Some(error),
            }),
        }
    }

    report
}

fn inspect_extraction_alias(
    entries: &mut Vec<ModelCacheEntry>,
    cache_root: &Path,
    alias: &ResolvedModelAlias,
    mode: ManifestValidation,
) -> Result<(), ModelLifecycleError> {
    let cache_dir = cache_root.join(&alias.cache_key);
    entries.push(inspect_extraction_cache_dir(
        cache_dir,
        &alias.cache_key,
        Some(alias),
        mode,
    ));
    inspect_extraction_temp_dirs(entries, cache_root, Some(&alias.cache_key))?;
    Ok(())
}

fn inspect_extraction_cache_root(
    entries: &mut Vec<ModelCacheEntry>,
    cache_root: &Path,
    mode: ManifestValidation,
) -> Result<(), ModelLifecycleError> {
    let read_dir = match fs::read_dir(cache_root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ModelLifecycleError::CacheInvalid {
                cache_dir: cache_root.display().to_string(),
                message: format!("read cache root: {error}"),
            });
        }
    };

    for entry in read_dir.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if extraction_temp_cache_key(&file_name).is_some() {
            continue;
        }
        if is_known_embedding_cache_key(&file_name) {
            continue;
        }
        let path = entry.path();
        if !path.join(MANIFEST_FILE_NAME).is_file() {
            continue;
        }
        match read_manifest(&path) {
            Ok(manifest) => match resolve_model_alias(&manifest.requested_alias) {
                Ok(alias) => entries.push(inspect_extraction_cache_dir(
                    path,
                    &alias.cache_key,
                    Some(&alias),
                    mode,
                )),
                Err(error) => entries.push(cache_entry(
                    CacheEntryParams::new(
                        ModelCacheFamily::Extraction,
                        file_name,
                        path,
                        ModelCacheState::Corrupted,
                        format!("manifest alias is invalid: {error}"),
                    )
                    .alias(Some(manifest.requested_alias))
                    .cleanup_eligible(true),
                )),
            },
            Err(error) => entries.push(cache_entry(
                CacheEntryParams::new(
                    ModelCacheFamily::Extraction,
                    file_name,
                    path,
                    ModelCacheState::Corrupted,
                    error,
                )
                .cleanup_eligible(true),
            )),
        }
    }

    inspect_extraction_temp_dirs(entries, cache_root, None)
}

fn inspect_extraction_cache_dir(
    path: PathBuf,
    cache_key: &str,
    alias: Option<&ResolvedModelAlias>,
    mode: ManifestValidation,
) -> ModelCacheEntry {
    if !path.exists() {
        return cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Extraction,
                cache_key,
                path,
                ModelCacheState::Missing,
                "cache directory is not present",
            )
            .alias(alias.map(|alias| alias.requested_alias.clone())),
        );
    }
    if !path.is_dir() {
        return cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Extraction,
                cache_key,
                path,
                ModelCacheState::Corrupted,
                "cache path is not a directory",
            )
            .alias(alias.map(|alias| alias.requested_alias.clone()))
            .cleanup_eligible(true),
        );
    }

    let validation = alias
        .ok_or_else(|| ModelLifecycleError::CacheInvalid {
            cache_dir: path.display().to_string(),
            message: "manifest alias is missing".to_owned(),
        })
        .and_then(|alias| ensure_cache_manifest(&path, alias, mode));
    match validation {
        Ok(manifest) => cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Extraction,
                cache_key,
                path,
                ModelCacheState::Complete,
                if mode == ManifestValidation::Full {
                    "manifest and file hashes verified"
                } else {
                    "manifest verified"
                },
            )
            .alias(Some(manifest.requested_alias))
            .complete_cache(true),
        ),
        Err(error) => {
            let state = cache_state_from_validation_error(&error);
            cache_entry(
                CacheEntryParams::new(
                    ModelCacheFamily::Extraction,
                    cache_key,
                    path,
                    state,
                    error.to_string(),
                )
                .alias(alias.map(|alias| alias.requested_alias.clone()))
                .cleanup_eligible(matches!(
                    state,
                    ModelCacheState::Incomplete | ModelCacheState::Corrupted
                )),
            )
        }
    }
}

fn inspect_extraction_temp_dirs(
    entries: &mut Vec<ModelCacheEntry>,
    cache_root: &Path,
    cache_key_filter: Option<&str>,
) -> Result<(), ModelLifecycleError> {
    let read_dir = match fs::read_dir(cache_root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ModelLifecycleError::CacheInvalid {
                cache_dir: cache_root.display().to_string(),
                message: format!("read cache root: {error}"),
            });
        }
    };
    let ttl = stale_temp_ttl();
    for entry in read_dir.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(cache_key) = extraction_temp_cache_key(&file_name) else {
            continue;
        };
        if cache_key_filter.is_some_and(|filter| filter != cache_key) {
            continue;
        }
        let active = download_temp_is_active(&entry.path(), ttl);
        let state = if active {
            ModelCacheState::ActiveTemporary
        } else {
            ModelCacheState::StaleTemporary
        };
        entries.push(cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Extraction,
                cache_key,
                entry.path(),
                state,
                if active {
                    "temporary download has a fresh heartbeat or mtime"
                } else {
                    "temporary download is older than the stale threshold"
                },
            )
            .cleanup_eligible(!active),
        ));
    }
    Ok(())
}

fn extraction_temp_cache_key(file_name: &str) -> Option<String> {
    let rest = file_name.strip_prefix('.')?;
    let (cache_key, suffix) = rest.rsplit_once("-download-")?;
    (!cache_key.is_empty() && !suffix.is_empty()).then(|| cache_key.to_owned())
}

#[cfg(feature = "online-model")]
fn inspect_embedding_model(
    entries: &mut Vec<ModelCacheEntry>,
    cache_root: &Path,
    model: &crate::core::inference::ModelConfig,
    include_missing: bool,
    verify_hashes: bool,
) {
    let cache_key = crate::core::inference::embedding_model_cache_key(model);
    let cache_dir = cache_root.join(&cache_key);
    if include_missing || cache_dir.exists() {
        entries.push(inspect_embedding_cache_dir(
            cache_dir.clone(),
            &cache_key,
            model,
            verify_hashes,
        ));
    }
    inspect_embedding_temp_files(entries, &cache_dir, &cache_key, model);
}

#[cfg(feature = "online-model")]
fn inspect_embedding_cache_dir(
    path: PathBuf,
    cache_key: &str,
    model: &crate::core::inference::ModelConfig,
    verify_hashes: bool,
) -> ModelCacheEntry {
    if !path.exists() {
        return cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Embedding,
                cache_key,
                path,
                ModelCacheState::Missing,
                "cache directory is not present",
            )
            .alias(Some(model.alias.clone())),
        );
    }
    if !path.is_dir() {
        return cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Embedding,
                cache_key,
                path,
                ModelCacheState::Corrupted,
                "cache path is not a directory",
            )
            .alias(Some(model.alias.clone()))
            .cleanup_eligible(true),
        );
    }

    match crate::core::inference::verify_embedding_model_cache(model, &path, verify_hashes) {
        Ok(()) => cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Embedding,
                cache_key,
                path,
                ModelCacheState::Complete,
                if verify_hashes {
                    "required files and file hashes verified"
                } else {
                    "required files are present"
                },
            )
            .alias(Some(model.alias.clone()))
            .complete_cache(true),
        ),
        Err(error) => {
            let state = if error.contains("missing required file") {
                ModelCacheState::Incomplete
            } else {
                ModelCacheState::Corrupted
            };
            cache_entry(
                CacheEntryParams::new(ModelCacheFamily::Embedding, cache_key, path, state, error)
                    .alias(Some(model.alias.clone()))
                    .cleanup_eligible(true),
            )
        }
    }
}

#[cfg(feature = "online-model")]
fn inspect_embedding_temp_files(
    entries: &mut Vec<ModelCacheEntry>,
    cache_dir: &Path,
    cache_key: &str,
    model: &crate::core::inference::ModelConfig,
) {
    let Ok(read_dir) = fs::read_dir(cache_dir) else {
        return;
    };
    let ttl = stale_temp_ttl();
    for entry in read_dir.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if !file_name.contains(".download-") {
            continue;
        }
        let active = recent_path_mtime(&entry.path(), ttl);
        let state = if active {
            ModelCacheState::ActiveTemporary
        } else {
            ModelCacheState::StaleTemporary
        };
        entries.push(cache_entry(
            CacheEntryParams::new(
                ModelCacheFamily::Embedding,
                cache_key,
                entry.path(),
                state,
                if active {
                    "temporary download file has a fresh mtime"
                } else {
                    "temporary download file is older than the stale threshold"
                },
            )
            .alias(Some(model.alias.clone()))
            .cleanup_eligible(!active),
        ));
    }
}

#[cfg(feature = "online-model")]
fn is_known_embedding_cache_key(cache_key: &str) -> bool {
    crate::core::inference::known_embedding_models()
        .iter()
        .any(|model| crate::core::inference::embedding_model_cache_key(model) == cache_key)
}

#[cfg(not(feature = "online-model"))]
fn is_known_embedding_cache_key(_cache_key: &str) -> bool {
    false
}

fn cache_state_from_validation_error(error: &ModelLifecycleError) -> ModelCacheState {
    let message = error.to_string().to_ascii_lowercase();
    if message.contains("missing")
        || message.contains("did not record")
        || message.contains("not a regular file")
        || message.contains("manifest.json is missing")
    {
        ModelCacheState::Incomplete
    } else {
        ModelCacheState::Corrupted
    }
}

struct CacheEntryParams {
    family: ModelCacheFamily,
    alias: Option<String>,
    cache_key: String,
    path: PathBuf,
    state: ModelCacheState,
    reason: String,
    cleanup_eligible: bool,
    complete_cache: bool,
}

impl CacheEntryParams {
    fn new(
        family: ModelCacheFamily,
        cache_key: impl Into<String>,
        path: PathBuf,
        state: ModelCacheState,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            family,
            alias: None,
            cache_key: cache_key.into(),
            path,
            state,
            reason: reason.into(),
            cleanup_eligible: false,
            complete_cache: false,
        }
    }

    fn alias(mut self, alias: Option<String>) -> Self {
        self.alias = alias;
        self
    }

    fn cleanup_eligible(mut self, cleanup_eligible: bool) -> Self {
        self.cleanup_eligible = cleanup_eligible;
        self
    }

    fn complete_cache(mut self, complete_cache: bool) -> Self {
        self.complete_cache = complete_cache;
        self
    }
}

fn cache_entry(params: CacheEntryParams) -> ModelCacheEntry {
    let CacheEntryParams {
        family,
        alias,
        cache_key,
        path,
        state,
        reason,
        cleanup_eligible,
        complete_cache,
    } = params;
    let (file_count, size_bytes, modified_unix) = path_tree_stats(&path);
    ModelCacheEntry {
        family,
        alias,
        cache_key,
        path,
        state,
        reason,
        file_count,
        size_bytes,
        modified_unix,
        cleanup_eligible,
        complete_cache,
    }
}

fn path_tree_stats(path: &Path) -> (usize, u64, Option<u64>) {
    let mut file_count = 0_usize;
    let mut size_bytes = 0_u64;
    let mut modified_unix = None;
    let mut stack = vec![path.to_path_buf()];

    while let Some(current) = stack.pop() {
        let Ok(metadata) = fs::symlink_metadata(&current) else {
            continue;
        };
        if let Some(modified) = metadata.modified().ok().and_then(system_time_secs_fallback) {
            modified_unix =
                Some(modified_unix.map_or(modified, |current: u64| current.max(modified)));
        }
        if metadata.is_file() {
            file_count += 1;
            size_bytes = size_bytes.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            let Ok(entries) = fs::read_dir(&current) else {
                continue;
            };
            stack.extend(entries.filter_map(Result::ok).map(|entry| entry.path()));
        }
    }

    (file_count, size_bytes, modified_unix)
}

fn path_is_within_cache_root(cache_root: &Path, path: &Path) -> bool {
    let Ok(root) = cache_root.canonicalize() else {
        return false;
    };
    match path.canonicalize() {
        Ok(target) => target != root && target.starts_with(&root),
        Err(_) => path
            .parent()
            .and_then(|parent| parent.canonicalize().ok())
            .is_some_and(|parent| parent == root || parent.starts_with(&root)),
    }
}

fn remove_cache_path(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("metadata {}: {error}", path.display())),
    };

    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path).map_err(|error| format!("remove {}: {error}", path.display()))
    } else {
        fs::remove_file(path).map_err(|error| format!("remove {}: {error}", path.display()))
    }
}

/// Returns a verified local cache directory without touching the network.
///
/// Runtime loaders must use this seam instead of `download_model()` so missing or corrupt
/// caches fail closed instead of silently fetching.
pub fn load_model_from_local_cache(alias: &str) -> Result<PathBuf, ModelLifecycleError> {
    let alias = resolve_model_alias(alias)?;
    let cache_dir = cache_dir_for_resolved_alias(&alias)?;
    if !cache_dir.is_dir() {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "no local model cache is present; run `quaid model pull {}` or `quaid extraction enable` first",
                alias.requested_alias
            ),
        });
    }
    verify_cache_manifest(&cache_dir, &alias).map_err(|error| match error {
        ModelLifecycleError::CacheInvalid { cache_dir, message } => {
            ModelLifecycleError::CacheInvalid {
                cache_dir,
                message: format!(
                    "{message}; re-run `quaid model pull {}` or `quaid extraction enable`",
                    alias.requested_alias
                ),
            }
        }
        other => other,
    })?;
    Ok(cache_dir)
}

/// Fetch a model into the local cache, reusing a verified existing cache
/// when present and otherwise downloading every required file with
/// progress events streamed through the supplied [`ProgressReporter`].
/// Returns [`ModelLifecycleError::DownloadsUnsupported`] when the build
/// does not include the `online-model` feature.
pub fn download_model(
    alias: &str,
    progress: &mut impl ProgressReporter,
) -> Result<PathBuf, ModelLifecycleError> {
    #[cfg(feature = "online-model")]
    {
        download_model_online(alias, progress)
    }

    #[cfg(not(feature = "online-model"))]
    {
        let _ = alias;
        let _ = progress;
        Err(ModelLifecycleError::DownloadsUnsupported)
    }
}

#[cfg(feature = "online-model")]
fn download_model_online(
    alias: &str,
    progress: &mut impl ProgressReporter,
) -> Result<PathBuf, ModelLifecycleError> {
    let alias = resolve_model_alias(alias)?;
    let cache_dir = cache_dir_for_resolved_alias(&alias)?;

    if cache_dir.is_dir() {
        if verify_cache_manifest(&cache_dir, &alias).is_ok() {
            progress.cache_hit(&cache_dir);
            return Ok(cache_dir);
        }
        fs::remove_dir_all(&cache_dir).map_err(|error| ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!("failed to remove stale cache before reinstall: {error}"),
        })?;
    }

    let cache_root = cache_dir
        .parent()
        .ok_or(ModelLifecycleError::CacheRootUnavailable)?;
    fs::create_dir_all(cache_root).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("create cache root {}: {error}", cache_root.display()),
    })?;
    scavenge_stale_download_dirs(cache_root, &alias.cache_key);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent(format!(
            "quaid/{}/model-lifecycle",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("build HTTP client: {error}"),
        })?;

    let metadata = source_pins_for_alias(&alias)
        .is_none()
        .then(|| fetch_model_metadata(&client, &alias))
        .transpose()?;
    let planned_file_count = source_pins_for_alias(&alias)
        .map(|files| files.len())
        .unwrap_or_else(|| {
            metadata
                .as_ref()
                .map(|metadata| select_files_to_download(metadata, &alias).map(|files| files.len()))
                .transpose()
                .ok()
                .flatten()
                .unwrap_or_default()
        });
    progress.planned(&alias, planned_file_count);

    let temp_dir = cache_root.join(format!(
        ".{}-download-{}-{}",
        alias.cache_key,
        download_timestamp_secs(),
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&temp_dir).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("create temporary cache {}: {error}", temp_dir.display()),
    })?;
    let mut temp_guard = TempDirCleanupGuard::new(temp_dir);
    touch_download_heartbeat(temp_guard.path());

    let install_result = match source_pins_for_alias(&alias) {
        Some(files) => install_source_pinned_model_into_dir(
            &client,
            &alias,
            files,
            temp_guard.path(),
            progress,
        ),
        None => {
            let Some(metadata) = metadata.as_ref() else {
                return Err(ModelLifecycleError::Metadata {
                    alias: alias.requested_alias.clone(),
                    message: "download metadata was not fetched for an unpinned alias".to_owned(),
                });
            };
            let files = select_files_to_download(metadata, &alias)?;
            install_model_into_dir(&client, &alias, &files, temp_guard.path(), progress)
        }
    };
    install_result?;

    if let Err(error) = fs::rename(temp_guard.path(), &cache_dir) {
        if cache_dir.is_dir() && verify_cache_manifest(&cache_dir, &alias).is_ok() {
            let _ = temp_guard.cleanup_now();
            progress.cache_hit(&cache_dir);
            return Ok(cache_dir);
        }
        return Err(ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!(
                "promote verified cache {} -> {}: {error}; run `quaid model clean {} --force` and retry `quaid model pull {}`",
                temp_guard.path().display(),
                cache_dir.display(),
                alias.requested_alias,
                alias.requested_alias
            ),
        });
    }
    temp_guard.disarm();

    Ok(cache_dir)
}

#[cfg(feature = "online-model")]
fn install_model_into_dir(
    client: &reqwest::blocking::Client,
    alias: &ResolvedModelAlias,
    files: &[String],
    temp_dir: &Path,
    progress: &mut impl ProgressReporter,
) -> Result<(), ModelLifecycleError> {
    let mut manifest_files = Vec::with_capacity(files.len());

    for relative_path in files {
        let artifact = download_artifact(client, alias, relative_path, temp_dir, progress)?;
        manifest_files.push(cached_file_from_artifact(temp_dir, artifact, alias)?);
    }

    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = CacheManifest {
        manifest_version: Some(MANIFEST_VERSION),
        requested_alias: alias.requested_alias.clone(),
        repo_id: alias.repo_id.clone(),
        revision: alias.revision.clone(),
        created_at_unix: Some(download_timestamp_secs()),
        files: manifest_files,
    };
    write_manifest(temp_dir, &manifest, alias)?;
    verify_cache_manifest_full(temp_dir, alias)?;
    Ok(())
}

#[cfg(feature = "online-model")]
fn install_source_pinned_model_into_dir(
    client: &reqwest::blocking::Client,
    alias: &ResolvedModelAlias,
    files: &[SourcePinnedFile],
    temp_dir: &Path,
    progress: &mut impl ProgressReporter,
) -> Result<(), ModelLifecycleError> {
    let mut manifest_files = Vec::with_capacity(files.len());

    for pinned_file in files {
        let artifact =
            download_source_pinned_artifact(client, alias, pinned_file, temp_dir, progress)?;
        manifest_files.push(cached_file_from_artifact(temp_dir, artifact, alias)?);
    }

    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = CacheManifest {
        manifest_version: Some(MANIFEST_VERSION),
        requested_alias: alias.requested_alias.clone(),
        repo_id: alias.repo_id.clone(),
        revision: alias.revision.clone(),
        created_at_unix: Some(download_timestamp_secs()),
        files: manifest_files,
    };
    write_manifest(temp_dir, &manifest, alias)?;
    verify_cache_manifest_full(temp_dir, alias)?;
    Ok(())
}

#[cfg(feature = "online-model")]
fn cached_file_from_artifact(
    cache_dir: &Path,
    artifact: DownloadedArtifact,
    alias: &ResolvedModelAlias,
) -> Result<CachedFile, ModelLifecycleError> {
    let path = cache_dir.join(&artifact.relative_path);
    let metadata = path
        .metadata()
        .map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("metadata {}: {error}", path.display()),
        })?;
    let modified_unix = metadata
        .modified()
        .ok()
        .and_then(system_time_secs)
        .unwrap_or_else(download_timestamp_secs);

    Ok(CachedFile {
        path: artifact.relative_path,
        sha256: artifact.sha256,
        size_bytes: Some(metadata.len()),
        modified_unix: Some(modified_unix),
        verified_from_source: artifact.verified_from_source,
    })
}

#[cfg(feature = "online-model")]
fn download_artifact(
    client: &reqwest::blocking::Client,
    alias: &ResolvedModelAlias,
    relative_path: &str,
    temp_dir: &Path,
    progress: &mut impl ProgressReporter,
) -> Result<DownloadedArtifact, ModelLifecycleError> {
    let relative_path = normalize_relative_path(relative_path).map_err(|message| {
        ModelLifecycleError::Metadata {
            alias: alias.requested_alias.clone(),
            message,
        }
    })?;
    let destination = temp_dir.join(&relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("create {}: {error}", parent.display()),
        })?;
    }

    let url = format!(
        "{}/{}/resolve/{}/{}",
        huggingface_base_url().trim_end_matches('/'),
        alias.repo_id,
        alias.revision.as_deref().unwrap_or("main"),
        relative_path
    );
    let mut response = client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("GET {url}: {error}"),
        })?;

    let total_bytes = response.content_length();
    progress.file_started(&relative_path, total_bytes);

    let expected_sha256 = expected_sha256_from_headers(response.headers());
    let mut file = File::create(&destination).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("create {}: {error}", destination.display()),
    })?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| ModelLifecycleError::Download {
                alias: alias.requested_alias.clone(),
                message: format!(
                    "read {url} for {relative_path}: {error} after {} received; retry with `quaid model pull {}`",
                    human_bytes(downloaded),
                    alias.requested_alias
                ),
            })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| ModelLifecycleError::Download {
                alias: alias.requested_alias.clone(),
                message: format!("write {}: {error}", destination.display()),
            })?;
        hasher.update(&buffer[..read]);
        downloaded += read as u64;
        touch_download_heartbeat(temp_dir);
        progress.file_progress(&relative_path, downloaded, total_bytes);
    }
    file.flush()
        .map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("flush {}: {error}", destination.display()),
        })?;

    let actual_sha256 = format!("{:x}", hasher.finalize());
    if let Some(expected_sha256) = expected_sha256.as_deref() {
        if actual_sha256 != expected_sha256 {
            let _ = fs::remove_file(&destination);
            return Err(ModelLifecycleError::Download {
                alias: alias.requested_alias.clone(),
                message: format!(
                    "integrity check failed for {}: expected SHA-256 {}, got {}; run `quaid model clean {} --force` and retry `quaid model pull {}`",
                    relative_path, expected_sha256, actual_sha256, alias.requested_alias, alias.requested_alias
                ),
            });
        }
    }

    progress.file_finished(&relative_path, &actual_sha256);
    Ok(DownloadedArtifact {
        relative_path,
        sha256: actual_sha256,
        verified_from_source: false,
    })
}

#[cfg(feature = "online-model")]
fn download_source_pinned_artifact(
    client: &reqwest::blocking::Client,
    alias: &ResolvedModelAlias,
    pinned_file: &SourcePinnedFile,
    temp_dir: &Path,
    progress: &mut impl ProgressReporter,
) -> Result<DownloadedArtifact, ModelLifecycleError> {
    let relative_path = normalize_relative_path(pinned_file.path).map_err(|message| {
        ModelLifecycleError::Metadata {
            alias: alias.requested_alias.clone(),
            message,
        }
    })?;
    let destination = temp_dir.join(&relative_path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("create {}: {error}", parent.display()),
        })?;
    }

    let url = format!(
        "{}/{}/resolve/{}/{}",
        huggingface_base_url().trim_end_matches('/'),
        alias.repo_id,
        alias.revision.as_deref().unwrap_or("main"),
        relative_path
    );
    let mut response = client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("GET {url}: {error}"),
        })?;

    let total_bytes = response.content_length();
    progress.file_started(&relative_path, total_bytes);

    let mut file = File::create(&destination).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("create {}: {error}", destination.display()),
    })?;
    let mut sha256 = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| ModelLifecycleError::Download {
                alias: alias.requested_alias.clone(),
                message: format!(
                    "read {url} for {relative_path}: {error} after {} received; retry with `quaid model pull {}`",
                    human_bytes(downloaded),
                    alias.requested_alias
                ),
            })?;
        if read == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buffer[..read]).map_err(|error| {
            ModelLifecycleError::Download {
                alias: alias.requested_alias.clone(),
                message: format!("write {}: {error}", destination.display()),
            }
        })?;
        sha256.update(&buffer[..read]);
        downloaded += read as u64;
        touch_download_heartbeat(temp_dir);
        progress.file_progress(&relative_path, downloaded, total_bytes);
    }
    std::io::Write::flush(&mut file).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("flush {}: {error}", destination.display()),
    })?;

    let actual_sha256 = format!("{:x}", sha256.finalize());
    verify_source_pin(
        &relative_path,
        downloaded,
        pinned_file.digest,
        &actual_sha256,
        &destination,
        alias,
    )?;

    progress.file_finished(&relative_path, &actual_sha256);
    Ok(DownloadedArtifact {
        relative_path,
        sha256: actual_sha256,
        verified_from_source: true,
    })
}

#[cfg(feature = "online-model")]
fn fetch_model_metadata(
    client: &reqwest::blocking::Client,
    alias: &ResolvedModelAlias,
) -> Result<ModelMetadata, ModelLifecycleError> {
    let base_url = huggingface_base_url();
    let base = base_url.trim_end_matches('/');
    let revision = alias.revision.as_deref();
    let candidate_urls = revision
        .map(|revision| {
            vec![
                format!("{base}/api/models/{}/revision/{revision}", alias.repo_id),
                format!("{base}/api/models/{}", alias.repo_id),
            ]
        })
        .unwrap_or_else(|| vec![format!("{base}/api/models/{}", alias.repo_id)]);

    let mut last_error = None;
    for url in candidate_urls {
        match client
            .get(&url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
        {
            Ok(response) => {
                let body = response
                    .text()
                    .map_err(|error| ModelLifecycleError::Metadata {
                        alias: alias.requested_alias.clone(),
                        message: format!("read metadata from {url}: {error}"),
                    })?;
                return serde_json::from_str::<ModelMetadata>(&body).map_err(|error| {
                    ModelLifecycleError::Metadata {
                        alias: alias.requested_alias.clone(),
                        message: format!("parse metadata from {url}: {error}"),
                    }
                });
            }
            Err(error) => last_error = Some(format!("{url}: {error}")),
        }
    }

    Err(ModelLifecycleError::Metadata {
        alias: alias.requested_alias.clone(),
        message: last_error.unwrap_or_else(|| "metadata request failed".to_owned()),
    })
}

#[cfg(feature = "online-model")]
fn download_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(feature = "online-model")]
fn system_time_secs(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

fn download_timestamp_secs_fallback() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn system_time_secs_fallback(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_secs())
}

#[cfg(feature = "online-model")]
fn stale_download_ttl() -> Duration {
    stale_temp_ttl()
}

fn stale_temp_ttl() -> Duration {
    std::env::var(STALE_CACHE_TTL_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_STALE_DOWNLOAD_TTL)
}

#[cfg(feature = "online-model")]
fn touch_download_heartbeat(temp_dir: &Path) {
    let heartbeat_path = temp_dir.join(DOWNLOAD_HEARTBEAT_FILE);
    let _ = fs::write(heartbeat_path, download_timestamp_secs().to_string());
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
fn source_pins_for_alias(alias: &ResolvedModelAlias) -> Option<&'static [SourcePinnedFile]> {
    match alias.requested_alias.to_ascii_lowercase().as_str() {
        "phi-3.5-mini" => Some(PHI_35_MINI_FILES),
        "gemma-3-1b" => Some(GEMMA_3_1B_FILES),
        "gemma-3-4b" => Some(GEMMA_3_4B_FILES),
        #[cfg(any(test, feature = "test-harness"))]
        TEST_PINNED_ALIAS => Some(TEST_CURATED_FILES),
        _ => None,
    }
}

#[cfg(feature = "online-model")]
fn scavenge_stale_download_dirs(cache_root: &Path, cache_key: &str) {
    let Ok(entries) = fs::read_dir(cache_root) else {
        return;
    };
    let now = download_timestamp_secs();
    let ttl = stale_download_ttl();
    for entry in entries.filter_map(Result::ok) {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let Some(created_at) = parse_download_timestamp(&file_name, cache_key, &entry.path())
        else {
            continue;
        };
        if download_temp_is_active(&entry.path(), ttl)
            || now.saturating_sub(created_at) < ttl.as_secs()
        {
            continue;
        }
        if let Err(error) = fs::remove_dir_all(entry.path()) {
            eprintln!(
                "Warning: failed to remove stale model download directory {}: {error}",
                entry.path().display()
            );
        }
    }
}

#[cfg(feature = "online-model")]
fn parse_download_timestamp(file_name: &str, cache_key: &str, path: &Path) -> Option<u64> {
    let prefix = format!(".{cache_key}-download-");
    let suffix = file_name.strip_prefix(&prefix)?;
    let timestamp = suffix
        .split_once('-')
        .and_then(|(timestamp, _)| timestamp.parse::<u64>().ok());
    if timestamp.is_some() {
        return timestamp;
    }
    path.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .filter(|age| *age >= stale_download_ttl())
        .map(|age| download_timestamp_secs().saturating_sub(age.as_secs()))
}

fn download_temp_is_active(path: &Path, ttl: Duration) -> bool {
    let heartbeat = path.join(DOWNLOAD_HEARTBEAT_FILE);
    recent_path_mtime(&heartbeat, ttl) || recent_path_mtime(path, ttl)
}

fn recent_path_mtime(path: &Path, ttl: Duration) -> bool {
    path.metadata()
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age < ttl)
}

#[cfg(feature = "online-model")]
fn verify_source_pin(
    relative_path: &str,
    byte_len: u64,
    digest: PinnedDigest,
    actual_sha256: &str,
    destination: &Path,
    alias: &ResolvedModelAlias,
) -> Result<(), ModelLifecycleError> {
    let (algorithm, expected, actual) = match digest {
        PinnedDigest::Sha256(expected) => ("SHA-256", expected, actual_sha256),
        PinnedDigest::GitBlobSha1(expected) => {
            let actual = git_blob_sha1_for_file(destination, byte_len).map_err(|message| {
                ModelLifecycleError::Download {
                    alias: alias.requested_alias.clone(),
                    message,
                }
            })?;
            return if actual == expected {
                Ok(())
            } else {
                let _ = fs::remove_file(destination);
                Err(ModelLifecycleError::Download {
                    alias: alias.requested_alias.clone(),
                    message: format!(
                        "integrity check failed for {}: expected git blob SHA-1 {}, got {}",
                        relative_path, expected, actual
                    ),
                })
            };
        }
    };

    if actual != expected {
        let _ = fs::remove_file(destination);
        return Err(ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!(
                "integrity check failed for {}: expected {} {}, got {}",
                relative_path, algorithm, expected, actual
            ),
        });
    }
    Ok(())
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
fn git_blob_sha1_for_file(path: &Path, byte_len: u64) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", byte_len).as_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(feature = "online-model")]
fn select_files_to_download(
    metadata: &ModelMetadata,
    alias: &ResolvedModelAlias,
) -> Result<Vec<String>, ModelLifecycleError> {
    let available: BTreeSet<String> = metadata
        .siblings
        .iter()
        .map(|sibling| sibling.rfilename.clone())
        .collect();

    for required in ["config.json", "tokenizer.json"] {
        if !available.contains(required) {
            return Err(ModelLifecycleError::Metadata {
                alias: alias.requested_alias.clone(),
                message: format!("required file `{required}` is missing"),
            });
        }
    }

    let mut selected = BTreeSet::new();
    selected.insert("config.json".to_owned());
    selected.insert("tokenizer.json".to_owned());

    for optional in OPTIONAL_SUPPORT_FILES {
        if available.contains(*optional) {
            selected.insert((*optional).to_owned());
        }
    }

    if available.contains("model.safetensors") {
        selected.insert("model.safetensors".to_owned());
    } else if available.contains("model.safetensors.index.json") {
        selected.insert("model.safetensors.index.json".to_owned());
        let shard_files = available
            .iter()
            .filter(|file_name| {
                file_name.starts_with("model-") && file_name.ends_with(".safetensors")
            })
            .cloned()
            .collect::<Vec<_>>();
        if shard_files.is_empty() {
            return Err(ModelLifecycleError::Metadata {
                alias: alias.requested_alias.clone(),
                message: "model.safetensors.index.json is present but no shard files were listed"
                    .to_owned(),
            });
        }
        selected.extend(shard_files);
    } else {
        return Err(ModelLifecycleError::Metadata {
            alias: alias.requested_alias.clone(),
            message: "no safetensors weights were listed in the model metadata".to_owned(),
        });
    }

    Ok(selected.into_iter().collect())
}

fn cache_dir_for_resolved_alias(
    alias: &ResolvedModelAlias,
) -> Result<PathBuf, ModelLifecycleError> {
    let cache_root = cache_root_dir()?;
    Ok(cache_root.join(&alias.cache_key))
}

fn cache_root_dir() -> Result<PathBuf, ModelLifecycleError> {
    if let Ok(path) = std::env::var(MODEL_CACHE_ROOT_ENV) {
        return Ok(PathBuf::from(path));
    }

    dirs::home_dir()
        .map(|home| home.join(".quaid").join("models"))
        .ok_or(ModelLifecycleError::CacheRootUnavailable)
}

fn sanitize_cache_key(alias: &str) -> String {
    alias
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn validate_repo_id(repo_id: &str) -> Result<(), String> {
    if repo_id.trim() != repo_id || repo_id.is_empty() {
        return Err("expected <org>/<model> without surrounding whitespace".to_owned());
    }

    if repo_id
        .chars()
        .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '\\' | '?' | '#'))
    {
        return Err("spaces, '\\\\', '#', and '?' are not allowed".to_owned());
    }

    let mut segments = repo_id.split('/');
    let Some(namespace) = segments.next() else {
        return Err("missing org segment".to_owned());
    };
    let Some(name) = segments.next() else {
        return Err("expected exactly one '/' separator".to_owned());
    };
    if segments.next().is_some() {
        return Err("expected exactly one '/' separator".to_owned());
    }
    if !valid_repo_segment(namespace) || !valid_repo_segment(name) {
        return Err("repo segments must be non-empty and cannot be '.' or '..'".to_owned());
    }
    Ok(())
}

fn valid_repo_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

fn normalize_relative_path(path: &str) -> Result<String, String> {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return Err(format!("absolute file paths are not allowed: {path}"));
    }
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(format!("parent-directory traversal is not allowed: {path}"));
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(format!("absolute file paths are not allowed: {path}"));
            }
        }
    }
    let normalized = normalized.to_string_lossy().replace('\\', "/");
    if normalized.is_empty() {
        Err("empty relative file path".to_owned())
    } else {
        Ok(normalized)
    }
}

fn verify_cache_manifest(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
) -> Result<(), ModelLifecycleError> {
    let _ = ensure_cache_manifest(cache_dir, alias, ManifestValidation::Fast)?;
    Ok(())
}

#[cfg(feature = "online-model")]
fn verify_cache_manifest_full(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
) -> Result<(), ModelLifecycleError> {
    let _ = ensure_cache_manifest(cache_dir, alias, ManifestValidation::Full)?;
    Ok(())
}

fn validated_manifest(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
) -> Result<CacheManifest, ModelLifecycleError> {
    ensure_cache_manifest(cache_dir, alias, ManifestValidation::Fast)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManifestValidation {
    Fast,
    Full,
}

fn ensure_cache_manifest(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
    mode: ManifestValidation,
) -> Result<CacheManifest, ModelLifecycleError> {
    if !cache_dir.join(MANIFEST_FILE_NAME).is_file() {
        return generate_manifest_from_trusted_sources(cache_dir, alias);
    }

    let manifest =
        read_manifest(cache_dir).map_err(|message| ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message,
        })?;
    let (manifest, should_upgrade) = validate_manifest_contents(cache_dir, alias, manifest, mode)?;
    if should_upgrade {
        try_upgrade_manifest(cache_dir, &manifest, alias);
    }
    Ok(manifest)
}

fn validate_manifest_contents(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
    mut manifest: CacheManifest,
    mode: ManifestValidation,
) -> Result<(CacheManifest, bool), ModelLifecycleError> {
    if manifest.requested_alias != alias.requested_alias {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "manifest alias mismatch: expected `{}`, found `{}`",
                alias.requested_alias, manifest.requested_alias
            ),
        });
    }
    if manifest.repo_id != alias.repo_id {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "manifest repo mismatch: expected `{}`, found `{}`",
                alias.repo_id, manifest.repo_id
            ),
        });
    }
    if manifest.revision != alias.revision {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "manifest revision mismatch: expected `{:?}`, found `{:?}`",
                alias.revision, manifest.revision
            ),
        });
    }
    if manifest
        .manifest_version
        .is_some_and(|version| version > MANIFEST_VERSION)
    {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "unsupported manifest version {:?}; upgrade Quaid or re-pull `{}`",
                manifest.manifest_version, alias.requested_alias
            ),
        });
    }
    if manifest.files.is_empty() {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: "manifest did not record any downloaded files".to_owned(),
        });
    }

    let mut should_upgrade =
        manifest.manifest_version != Some(MANIFEST_VERSION) || manifest.created_at_unix.is_none();
    if manifest.created_at_unix.is_none() {
        manifest.created_at_unix = Some(download_timestamp_secs_fallback());
    }

    for file in &mut manifest.files {
        let relative_path = normalize_relative_path(&file.path).map_err(|message| {
            ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!("manifest contains invalid path `{}`: {message}", file.path),
            }
        })?;
        let path = cache_dir.join(relative_path);
        let metadata = path
            .metadata()
            .map_err(|error| ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!("metadata {}: {error}", path.display()),
            })?;
        if !metadata.is_file() {
            return Err(ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!("manifest path {} is not a regular file", file.path),
            });
        }
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(system_time_secs_fallback)
            .unwrap_or_else(download_timestamp_secs_fallback);
        let metadata_mismatch =
            file.size_bytes != Some(metadata.len()) || file.modified_unix != Some(modified_unix);
        let must_hash = mode == ManifestValidation::Full || metadata_mismatch;
        if must_hash {
            let actual =
                file_sha256(&path).map_err(|message| ModelLifecycleError::CacheInvalid {
                    cache_dir: cache_dir.display().to_string(),
                    message,
                })?;
            if actual != file.sha256 {
                return Err(ModelLifecycleError::CacheInvalid {
                    cache_dir: cache_dir.display().to_string(),
                    message: format!(
                        "file hash mismatch for {}: expected {}, got {}; run `quaid model clean {} --force` and retry `quaid model pull {}`",
                        file.path, file.sha256, actual, alias.requested_alias, alias.requested_alias
                    ),
                });
            }
        }
        if metadata_mismatch {
            should_upgrade = true;
            file.size_bytes = Some(metadata.len());
            file.modified_unix = Some(modified_unix);
        }
    }

    manifest.manifest_version = Some(MANIFEST_VERSION);
    Ok((manifest, should_upgrade))
}

fn generate_manifest_from_trusted_sources(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
) -> Result<CacheManifest, ModelLifecycleError> {
    #[cfg(any(feature = "online-model", test, feature = "test-harness"))]
    {
        if let Some(files) = source_pins_for_alias(alias) {
            let manifest = manifest_from_source_pins(cache_dir, alias, files)?;
            try_upgrade_manifest(cache_dir, &manifest, alias);
            return Ok(manifest);
        }
    }

    Err(ModelLifecycleError::CacheInvalid {
        cache_dir: cache_dir.display().to_string(),
        message: format!(
            "manifest.json is missing and `{}` has no trusted source pins; run `quaid model clean {} --force` and retry `quaid model pull {}`",
            alias.requested_alias, alias.requested_alias, alias.requested_alias
        ),
    })
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
fn manifest_from_source_pins(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
    files: &[SourcePinnedFile],
) -> Result<CacheManifest, ModelLifecycleError> {
    let mut manifest_files = Vec::with_capacity(files.len());
    for pinned_file in files {
        let relative_path = normalize_relative_path(pinned_file.path).map_err(|message| {
            ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message,
            }
        })?;
        let path = cache_dir.join(&relative_path);
        let metadata = path
            .metadata()
            .map_err(|error| ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!("metadata {}: {error}", path.display()),
            })?;
        let actual_sha256 =
            file_sha256(&path).map_err(|message| ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message,
            })?;
        verify_source_pin_for_cache(
            &relative_path,
            metadata.len(),
            pinned_file.digest,
            &actual_sha256,
            &path,
            alias,
            cache_dir,
        )?;
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(system_time_secs_fallback)
            .unwrap_or_else(download_timestamp_secs_fallback);
        manifest_files.push(CachedFile {
            path: relative_path,
            sha256: actual_sha256,
            size_bytes: Some(metadata.len()),
            modified_unix: Some(modified_unix),
            verified_from_source: true,
        });
    }
    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(CacheManifest {
        manifest_version: Some(MANIFEST_VERSION),
        requested_alias: alias.requested_alias.clone(),
        repo_id: alias.repo_id.clone(),
        revision: alias.revision.clone(),
        created_at_unix: Some(download_timestamp_secs_fallback()),
        files: manifest_files,
    })
}

#[cfg(any(feature = "online-model", test, feature = "test-harness"))]
fn verify_source_pin_for_cache(
    relative_path: &str,
    byte_len: u64,
    digest: PinnedDigest,
    actual_sha256: &str,
    path: &Path,
    alias: &ResolvedModelAlias,
    cache_dir: &Path,
) -> Result<(), ModelLifecycleError> {
    match digest {
        PinnedDigest::Sha256(expected) if actual_sha256 == expected => Ok(()),
        PinnedDigest::Sha256(expected) => Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: format!(
                "file hash mismatch for {relative_path}: expected SHA-256 {expected}, got {actual_sha256}; run `quaid model clean {} --force` and retry `quaid model pull {}`",
                alias.requested_alias, alias.requested_alias
            ),
        }),
        PinnedDigest::GitBlobSha1(expected) => {
            let actual = git_blob_sha1_for_file(path, byte_len).map_err(|message| {
                ModelLifecycleError::CacheInvalid {
                    cache_dir: cache_dir.display().to_string(),
                    message,
                }
            })?;
            if actual == expected {
                Ok(())
            } else {
                Err(ModelLifecycleError::CacheInvalid {
                    cache_dir: cache_dir.display().to_string(),
                    message: format!(
                        "file hash mismatch for {relative_path}: expected git blob SHA-1 {expected}, got {actual}; run `quaid model clean {} --force` and retry `quaid model pull {}`",
                        alias.requested_alias, alias.requested_alias
                    ),
                })
            }
        }
    }
}

fn try_upgrade_manifest(cache_dir: &Path, manifest: &CacheManifest, alias: &ResolvedModelAlias) {
    #[cfg(feature = "online-model")]
    if let Err(error) = write_manifest(cache_dir, manifest, alias) {
        eprintln!(
            "Warning: failed to write upgraded manifest for `{}` at {}: {error}",
            alias.requested_alias,
            cache_dir.display()
        );
    }
    #[cfg(not(feature = "online-model"))]
    {
        let _ = cache_dir;
        let _ = manifest;
        let _ = alias;
    }
}

#[cfg(feature = "online-model")]
fn write_manifest(
    cache_dir: &Path,
    manifest: &CacheManifest,
    alias: &ResolvedModelAlias,
) -> Result<(), ModelLifecycleError> {
    let path = cache_dir.join(MANIFEST_FILE_NAME);
    let manifest_json =
        serde_json::to_vec_pretty(manifest).map_err(|error| ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!("serialize {}: {error}", path.display()),
        })?;
    fs::write(&path, manifest_json).map_err(|error| ModelLifecycleError::Download {
        alias: alias.requested_alias.clone(),
        message: format!("write {}: {error}", path.display()),
    })
}

fn read_manifest(cache_dir: &Path) -> Result<CacheManifest, String> {
    let path = cache_dir.join(MANIFEST_FILE_NAME);
    let bytes = fs::read(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|error| format!("parse {}: {error}", path.display()))
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(feature = "online-model")]
fn huggingface_base_url() -> String {
    std::env::var(HUGGINGFACE_BASE_URL_ENV)
        .unwrap_or_else(|_| DEFAULT_HUGGINGFACE_BASE_URL.to_owned())
}

#[cfg(feature = "online-model")]
fn expected_sha256_from_headers(headers: &reqwest::header::HeaderMap) -> Option<String> {
    for header_name in [
        "x-sha256",
        "x-linked-etag",
        reqwest::header::ETAG.as_str(),
        "x-xet-hash",
    ] {
        if let Some(header_value) = headers.get(header_name) {
            let raw = header_value.to_str().ok()?.trim().trim_matches('"');
            if raw.len() == 64 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
                return Some(raw.to_ascii_lowercase());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = self.previous.as_ref() {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn write_test_pinned_files(cache_dir: &Path) {
        std::fs::create_dir_all(cache_dir).expect("cache dir");
        std::fs::write(cache_dir.join("config.json"), br#"{"model_type":"phi3"}"#).expect("config");
        std::fs::write(cache_dir.join("model.safetensors"), b"tiny-test-weights").expect("weights");
        std::fs::write(cache_dir.join("tokenizer.json"), br#"{"version":"1.0"}"#)
            .expect("tokenizer");
    }

    fn write_manifest(cache_dir: &Path, manifest: &CacheManifest) {
        let bytes = serde_json::to_vec(manifest).expect("serialize manifest");
        std::fs::write(cache_dir.join(MANIFEST_FILE_NAME), bytes).expect("write manifest");
    }

    #[test]
    fn resolve_model_alias_maps_standard_aliases_to_pinned_repos() {
        let phi = resolve_model_alias("phi-3.5-mini").expect("resolve phi");
        assert_eq!(phi.repo_id, "microsoft/Phi-3.5-mini-instruct");
        assert_eq!(
            phi.revision.as_deref(),
            Some("2fe192450127e6a83f7441aef6e3ca586c338b77")
        );

        let gemma = resolve_model_alias("gemma-3-1b").expect("resolve gemma");
        assert_eq!(gemma.repo_id, "google/gemma-3-1b-it");
        assert_eq!(
            gemma.revision.as_deref(),
            Some("dcc83ea841ab6100d6b47a070329e1ba4cf78752")
        );
    }

    #[test]
    fn source_pins_cover_curated_aliases() {
        let phi = resolve_model_alias("phi-3.5-mini").expect("resolve phi");
        let gemma_1b = resolve_model_alias("gemma-3-1b").expect("resolve gemma 1b");
        let gemma_4b = resolve_model_alias("gemma-3-4b").expect("resolve gemma 4b");

        assert_eq!(
            source_pins_for_alias(&phi).map(|files| files.len()),
            Some(10)
        );
        assert_eq!(
            source_pins_for_alias(&gemma_1b).map(|files| files.len()),
            Some(8)
        );
        assert_eq!(
            source_pins_for_alias(&gemma_4b).map(|files| files.len()),
            Some(13)
        );
    }

    #[test]
    fn resolve_model_alias_passes_through_raw_repo_ids() {
        let resolved = resolve_model_alias("org/custom-model").expect("resolve raw repo");
        assert_eq!(resolved.repo_id, "org/custom-model");
        assert!(resolved.revision.is_none());
        assert_eq!(resolved.cache_key, "org-custom-model");
    }

    #[test]
    fn resolve_model_alias_rejects_invalid_repo_ids() {
        let error = resolve_model_alias("org/model?rev=main").expect_err("reject invalid repo");
        assert!(error.to_string().contains("invalid model alias"));
    }

    #[test]
    fn normalize_relative_path_rejects_parent_directory_traversal() {
        let error = normalize_relative_path("../model.safetensors").expect_err("reject traversal");
        assert!(error.contains("parent-directory traversal"));
    }

    #[test]
    fn sanitize_cache_key_normalizes_path_separators() {
        assert_eq!(sanitize_cache_key("org/model"), "org-model");
        assert_eq!(sanitize_cache_key("phi-3.5-mini"), "phi-3.5-mini");
    }

    #[serial_test::serial]
    #[test]
    fn inspect_model_caches_reports_missing_alias_and_temporary_entries() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let _cache_root = EnvVarGuard::set(MODEL_CACHE_ROOT_ENV, cache_root.path());
        std::fs::create_dir(cache_root.path().join(".phi-3.5-mini-download-active"))
            .expect("temp download dir");

        let entries = inspect_model_caches(Some("phi-3.5-mini"), false).expect("inspect caches");

        assert!(entries.iter().any(|entry| {
            entry.family == ModelCacheFamily::Extraction
                && entry.state == ModelCacheState::Missing
                && entry.cache_key == "phi-3.5-mini"
        }));
        assert!(entries.iter().any(|entry| {
            entry.family == ModelCacheFamily::Extraction
                && entry.state == ModelCacheState::ActiveTemporary
                && !entry.cleanup_eligible
        }));
    }

    #[serial_test::serial]
    #[test]
    fn trusted_source_pin_cache_loads_status_and_root_inspection() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let _cache_root = EnvVarGuard::set(MODEL_CACHE_ROOT_ENV, cache_root.path());
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve test alias");
        let cache_dir = cache_root.path().join(&alias.cache_key);
        write_test_pinned_files(&cache_dir);

        assert_eq!(
            load_model_from_local_cache(TEST_PINNED_ALIAS).expect("load source-pinned cache"),
            cache_dir
        );

        let manifest = manifest_from_source_pins(&cache_dir, &alias, TEST_CURATED_FILES)
            .expect("source-pinned manifest");
        write_manifest(&cache_dir, &manifest);
        let status = cached_model_status(TEST_PINNED_ALIAS).expect("cached status");
        assert!(status.is_cached);
        assert!(status.verified);
        assert!(status.source_pinned);

        let entries = inspect_model_caches(None, true).expect("inspect root");
        assert!(entries.iter().any(|entry| {
            entry.cache_key == alias.cache_key
                && entry.state == ModelCacheState::Complete
                && entry.complete_cache
        }));
    }

    #[test]
    fn corrupted_and_incomplete_extraction_cache_entries_are_classified() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let alias = resolve_model_alias("phi-3.5-mini").expect("resolve alias");
        std::fs::write(cache_root.path().join("phi-3.5-mini"), b"not a directory")
            .expect("file cache path");
        let file_entry = inspect_extraction_cache_dir(
            cache_root.path().join("phi-3.5-mini"),
            "phi-3.5-mini",
            Some(&alias),
            ManifestValidation::Fast,
        );
        assert_eq!(file_entry.state, ModelCacheState::Corrupted);
        assert!(file_entry.cleanup_eligible);

        let broken_manifest_dir = cache_root.path().join("broken");
        std::fs::create_dir(&broken_manifest_dir).expect("broken dir");
        std::fs::write(
            broken_manifest_dir.join(MANIFEST_FILE_NAME),
            b"{not valid json",
        )
        .expect("broken manifest");
        let mut entries = Vec::new();
        inspect_extraction_cache_root(&mut entries, cache_root.path(), ManifestValidation::Fast)
            .expect("inspect root");
        assert!(entries.iter().any(|entry| {
            entry.cache_key == "broken"
                && entry.state == ModelCacheState::Corrupted
                && entry.cleanup_eligible
        }));
    }

    #[test]
    fn manifest_validation_rejects_mismatches_and_hash_drift() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve test alias");
        let cache_dir = cache_root.path().join(&alias.cache_key);
        write_test_pinned_files(&cache_dir);
        let manifest = manifest_from_source_pins(&cache_dir, &alias, TEST_CURATED_FILES)
            .expect("source-pinned manifest");

        let mut wrong_alias = manifest.clone();
        wrong_alias.requested_alias = "phi-3.5-mini".to_owned();
        assert!(validate_manifest_contents(
            &cache_dir,
            &alias,
            wrong_alias,
            ManifestValidation::Fast,
        )
        .unwrap_err()
        .to_string()
        .contains("alias mismatch"));

        let mut empty_manifest = manifest.clone();
        empty_manifest.files.clear();
        assert!(validate_manifest_contents(
            &cache_dir,
            &alias,
            empty_manifest,
            ManifestValidation::Fast,
        )
        .unwrap_err()
        .to_string()
        .contains("did not record"));

        let mut wrong_hash = manifest;
        wrong_hash.files[0].sha256 = "0".repeat(64);
        assert!(validate_manifest_contents(
            &cache_dir,
            &alias,
            wrong_hash,
            ManifestValidation::Full,
        )
        .unwrap_err()
        .to_string()
        .contains("hash mismatch"));

        assert_eq!(
            cache_state_from_validation_error(&ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: "manifest.json is missing".to_owned(),
            }),
            ModelCacheState::Incomplete
        );
    }

    #[serial_test::serial]
    #[test]
    fn utility_helpers_cover_edge_paths() {
        assert!(resolve_model_alias("   ").is_err());
        assert!(resolve_model_alias("org/model/extra").is_err());
        assert_eq!(extraction_temp_cache_key("plain"), None);
        assert_eq!(extraction_temp_cache_key(".model-download-"), None);
        assert_eq!(stale_temp_ttl(), DEFAULT_STALE_DOWNLOAD_TTL);

        let _ttl = EnvVarGuard::set(STALE_CACHE_TTL_ENV, "3");
        assert_eq!(stale_temp_ttl(), Duration::from_secs(3));

        let file = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(file.path(), b"abc").expect("write file");
        let blob_sha = git_blob_sha1_for_file(file.path(), 3).expect("blob sha");
        assert_eq!(blob_sha, "f2ba8f84ab5c1bce84a7b441cb1959cfc7093b7f");

        assert!(path_is_within_cache_root(
            file.path().parent().unwrap(),
            &file.path().parent().unwrap().join("future")
        ));
        assert!(remove_cache_path(&file.path().parent().unwrap().join("missing")).is_ok());
    }

    #[test]
    fn progress_reporters_and_download_unsupported_paths_are_covered() {
        let alias = resolve_model_alias("phi-3.5-mini").expect("resolve alias");
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let cache_dir = cache_root.path().join("phi-3.5-mini");
        let mut reporter = ConsoleProgressReporter::default();
        reporter.planned(&alias, 2);
        reporter.cache_hit(&cache_dir);
        reporter.file_started("config.json", Some(1536));
        reporter.file_progress("config.json", 1536, Some(1536));
        reporter.file_finished("config.json", "abcdef1234567890");
        reporter.file_started("tokenizer.json", None);
        reporter.file_progress("tokenizer.json", 42, None);
        reporter.file_finished("tokenizer.json", "abc");

        let mut noop = NoopProgressReporter;
        noop.planned(&alias, 0);
        noop.cache_hit(&cache_dir);
        noop.file_started("file", None);
        noop.file_progress("file", 1, None);
        noop.file_finished("file", "sha");

        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_duration(Duration::from_millis(250)), "0s");
        assert_eq!(human_duration(Duration::from_secs(2)), "2s");
        if !cfg!(feature = "online-model") {
            assert!(matches!(
                download_model("phi-3.5-mini", &mut noop),
                Err(ModelLifecycleError::DownloadsUnsupported)
            ));
        }
    }

    #[serial_test::serial]
    #[test]
    fn lifecycle_edge_paths_cover_validation_and_cleanup_branches() {
        let alias = resolve_model_alias("org/model").expect("resolve raw alias");
        assert!(cache_dir_for_alias("org/model").is_ok());
        assert!(resolve_model_alias("org/model#bad").is_err());
        assert!(validate_repo_id(" org/model").is_err());
        assert!(validate_repo_id("org/model?rev=main").is_err());
        assert!(validate_repo_id("org/model/extra").is_err());
        assert!(validate_repo_id("./model").is_err());
        assert!(validate_repo_id("org/..").is_err());
        assert!(normalize_relative_path("/absolute.bin").is_err());
        assert!(normalize_relative_path(".").is_err());
        assert_eq!(normalize_relative_path("a/./b.bin").unwrap(), "a/b.bin");

        let mut reporter = ConsoleProgressReporter::default();
        reporter.planned(&alias, 1);
        reporter.file_started("weights.bin", Some(4096));
        reporter.last_progress_at = Some(Instant::now());
        reporter.file_progress("weights.bin", 1024, Some(4096));
        reporter.current_file_started_at = Some(Instant::now() - Duration::from_secs(2));
        reporter.last_progress_at = Some(Instant::now() - Duration::from_secs(2));
        reporter.file_progress("weights.bin", 1024, Some(4096));
        assert_eq!(human_duration(Duration::from_secs(61)), "1m01s");
        assert_eq!(human_duration(Duration::from_secs(3661)), "1h01m");

        let cache_root = tempfile::TempDir::new().expect("cache root");
        let _cache_root = EnvVarGuard::set(MODEL_CACHE_ROOT_ENV, cache_root.path());
        let malformed = cache_root.path().join("malformed");
        std::fs::create_dir(&malformed).expect("malformed dir");
        write_manifest(
            &malformed,
            &CacheManifest {
                manifest_version: Some(MANIFEST_VERSION),
                requested_alias: "bad alias?".to_owned(),
                repo_id: "org/model".to_owned(),
                revision: None,
                created_at_unix: Some(1),
                files: vec![CachedFile {
                    path: "config.json".to_owned(),
                    sha256: "abc".to_owned(),
                    size_bytes: Some(0),
                    modified_unix: Some(1),
                    verified_from_source: false,
                }],
            },
        );
        let entries = inspect_model_caches(None, false).expect("inspect malformed root");
        assert!(entries.iter().any(|entry| {
            entry.cache_key == "malformed" && entry.state == ModelCacheState::Corrupted
        }));

        let existing = cache_root.path().join("existing");
        std::fs::create_dir(&existing).expect("existing dir");
        let missing_alias_entry = inspect_extraction_cache_dir(
            existing.clone(),
            "existing",
            None,
            ManifestValidation::Fast,
        );
        assert_eq!(missing_alias_entry.state, ModelCacheState::Incomplete);

        let file_cache_root = tempfile::NamedTempFile::new().expect("file cache root");
        let _file_cache_root = EnvVarGuard::set(MODEL_CACHE_ROOT_ENV, file_cache_root.path());
        assert!(inspect_model_caches(None, false).is_err());
        let report = remove_model_cache_entries(&[ModelCacheEntry {
            family: ModelCacheFamily::Extraction,
            alias: None,
            cache_key: "missing-root".to_owned(),
            path: cache_root.path().join("missing-root"),
            state: ModelCacheState::StaleTemporary,
            reason: "missing root".to_owned(),
            file_count: 0,
            size_bytes: 0,
            modified_unix: None,
            cleanup_eligible: true,
            complete_cache: false,
        }]);
        assert_eq!(report.failed.len(), 1);
        drop(_file_cache_root);

        let removable_file = cache_root.path().join("removable-file");
        std::fs::write(&removable_file, b"x").expect("removable file");
        assert!(remove_cache_path(&removable_file).is_ok());
        assert!(!removable_file.exists());
        assert!(!path_is_within_cache_root(
            &cache_root.path().join("does-not-exist"),
            &existing
        ));

        let pin_error = verify_source_pin_for_cache(
            "weights.bin",
            3,
            PinnedDigest::Sha256("0000"),
            "1111",
            &existing.join("missing.bin"),
            &alias,
            &existing,
        )
        .expect_err("sha mismatch");
        assert!(pin_error.to_string().contains("SHA-256"));
        std::fs::write(existing.join("blob.bin"), b"abc").expect("blob");
        let pin_error = verify_source_pin_for_cache(
            "blob.bin",
            3,
            PinnedDigest::GitBlobSha1("0000"),
            "unused",
            &existing.join("blob.bin"),
            &alias,
            &existing,
        )
        .expect_err("git blob mismatch");
        assert!(pin_error.to_string().contains("git blob"));
    }

    #[serial_test::serial]
    #[test]
    fn remove_model_cache_entries_removes_only_allowed_paths() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let _cache_root = EnvVarGuard::set(MODEL_CACHE_ROOT_ENV, cache_root.path());
        let removable = cache_root.path().join("stale");
        std::fs::create_dir(&removable).expect("stale dir");
        std::fs::write(removable.join("file.bin"), [1_u8, 2, 3]).expect("cache file");
        let outside = tempfile::NamedTempFile::new().expect("outside file");
        let protected = cache_root.path().join("protected");
        std::fs::create_dir(&protected).expect("protected dir");

        let report = remove_model_cache_entries(&[
            ModelCacheEntry {
                family: ModelCacheFamily::Extraction,
                alias: None,
                cache_key: "stale".to_owned(),
                path: removable.clone(),
                state: ModelCacheState::StaleTemporary,
                reason: "stale".to_owned(),
                file_count: 1,
                size_bytes: 3,
                modified_unix: None,
                cleanup_eligible: true,
                complete_cache: false,
            },
            ModelCacheEntry {
                family: ModelCacheFamily::Extraction,
                alias: None,
                cache_key: "outside".to_owned(),
                path: outside.path().to_path_buf(),
                state: ModelCacheState::StaleTemporary,
                reason: "outside".to_owned(),
                file_count: 1,
                size_bytes: 1,
                modified_unix: None,
                cleanup_eligible: true,
                complete_cache: false,
            },
            ModelCacheEntry {
                family: ModelCacheFamily::Extraction,
                alias: None,
                cache_key: "protected".to_owned(),
                path: protected,
                state: ModelCacheState::ActiveTemporary,
                reason: "active".to_owned(),
                file_count: 0,
                size_bytes: 0,
                modified_unix: None,
                cleanup_eligible: false,
                complete_cache: false,
            },
        ]);

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.bytes_freed, 3);
        assert_eq!(report.failed.len(), 2);
        assert!(!removable.exists());
    }

    #[test]
    fn path_tree_stats_counts_nested_files_and_cache_root_checks_boundaries() {
        let cache_root = tempfile::TempDir::new().expect("cache root");
        let nested = cache_root.path().join("entry").join("nested");
        std::fs::create_dir_all(&nested).expect("nested");
        std::fs::write(cache_root.path().join("entry").join("a.bin"), [1_u8, 2]).expect("file a");
        std::fs::write(nested.join("b.bin"), [3_u8]).expect("file b");

        let (file_count, size_bytes, modified_unix) =
            path_tree_stats(&cache_root.path().join("entry"));

        assert_eq!(file_count, 2);
        assert_eq!(size_bytes, 3);
        assert!(modified_unix.is_some());
        assert!(path_is_within_cache_root(
            cache_root.path(),
            &cache_root.path().join("entry")
        ));
        assert!(!path_is_within_cache_root(
            cache_root.path(),
            cache_root.path()
        ));
    }

    // --- verify_source_pin unit tests (require online-model feature for the fn) ---

    #[cfg(feature = "online-model")]
    #[test]
    fn verify_source_pin_accepts_sha256_match() {
        let content = b"tiny-test-weights";
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(tmp.path(), content).expect("write");
        let actual_sha256 = format!("{:x}", Sha256::digest(content));
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve");
        verify_source_pin(
            "model.safetensors",
            content.len() as u64,
            PinnedDigest::Sha256(
                "7f5bb0fd7b2f570f5c9140e319c619275a0b4960f8c64facf1f59b39511e1d6f",
            ),
            &actual_sha256,
            tmp.path(),
            &alias,
        )
        .expect("SHA-256 pin should pass for matching content");
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn verify_source_pin_rejects_sha256_mismatch() {
        let content = b"wrong-model-weights";
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(tmp.path(), content).expect("write");
        let actual_sha256 = format!("{:x}", Sha256::digest(content));
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve");
        let err = verify_source_pin(
            "model.safetensors",
            content.len() as u64,
            PinnedDigest::Sha256(
                "7f5bb0fd7b2f570f5c9140e319c619275a0b4960f8c64facf1f59b39511e1d6f",
            ),
            &actual_sha256,
            tmp.path(),
            &alias,
        )
        .expect_err("SHA-256 pin should reject non-matching content");
        assert!(
            err.to_string().contains("integrity check failed"),
            "unexpected error: {err}"
        );
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn verify_source_pin_accepts_git_blob_sha1_match() {
        let content = br#"{"model_type":"phi3"}"#;
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(tmp.path(), content).expect("write");
        let actual_sha256 = format!("{:x}", Sha256::digest(content));
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve");
        verify_source_pin(
            "config.json",
            content.len() as u64,
            PinnedDigest::GitBlobSha1("3156de0790e76b33247de6adc66765c4fc608b42"),
            &actual_sha256,
            tmp.path(),
            &alias,
        )
        .expect("git-blob-SHA1 pin should pass for matching content");
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn verify_source_pin_rejects_git_blob_sha1_mismatch() {
        let content = b"wrong-config";
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(tmp.path(), content).expect("write");
        let actual_sha256 = format!("{:x}", Sha256::digest(content));
        let alias = resolve_model_alias(TEST_PINNED_ALIAS).expect("resolve");
        let err = verify_source_pin(
            "config.json",
            content.len() as u64,
            PinnedDigest::GitBlobSha1("3156de0790e76b33247de6adc66765c4fc608b42"),
            &actual_sha256,
            tmp.path(),
            &alias,
        )
        .expect_err("git-blob-SHA1 pin should reject non-matching content");
        assert!(
            err.to_string().contains("integrity check failed"),
            "unexpected error: {err}"
        );
    }
}
