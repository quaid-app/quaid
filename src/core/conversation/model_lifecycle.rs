use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use thiserror::Error;

const DEFAULT_HUGGINGFACE_BASE_URL: &str = "https://huggingface.co";
const MODEL_CACHE_ROOT_ENV: &str = "QUAID_MODEL_CACHE_DIR";
const HUGGINGFACE_BASE_URL_ENV: &str = "QUAID_HF_BASE_URL";
const MANIFEST_FILE_NAME: &str = "manifest.json";
const STALE_DOWNLOAD_TTL: Duration = Duration::from_secs(6 * 60 * 60);

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
    fn planned(&mut self, _alias: &ResolvedModelAlias, _file_count: usize) {}

    fn cache_hit(&mut self, _cache_dir: &Path) {}

    fn file_started(&mut self, _file_name: &str, _bytes_total: Option<u64>) {}

    fn file_progress(
        &mut self,
        _file_name: &str,
        _downloaded_bytes: u64,
        _bytes_total: Option<u64>,
    ) {
    }

    fn file_finished(&mut self, _file_name: &str, _actual_sha256: &str) {}
}

/// Human-readable stderr reporter for CLI commands.
#[derive(Debug, Default)]
pub struct ConsoleProgressReporter;

impl ProgressReporter for ConsoleProgressReporter {
    fn planned(&mut self, alias: &ResolvedModelAlias, file_count: usize) {
        eprintln!(
            "Downloading model `{}` from {} ({} file(s))",
            alias.requested_alias, alias.repo_id, file_count
        );
    }

    fn cache_hit(&mut self, cache_dir: &Path) {
        eprintln!("Model cache already verified at {}", cache_dir.display());
    }

    fn file_started(&mut self, file_name: &str, bytes_total: Option<u64>) {
        match bytes_total {
            Some(bytes_total) => eprintln!("  → {file_name} ({bytes_total} bytes)"),
            None => eprintln!("  → {file_name}"),
        }
    }

    fn file_finished(&mut self, file_name: &str, _actual_sha256: &str) {
        eprintln!("  ✓ {file_name}");
    }
}

/// Silent reporter for tests and non-interactive callers.
#[derive(Debug, Default)]
pub struct NoopProgressReporter;

impl ProgressReporter for NoopProgressReporter {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedModelAlias {
    pub requested_alias: String,
    pub cache_key: String,
    pub repo_id: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedModelStatus {
    pub alias: ResolvedModelAlias,
    pub cache_dir: PathBuf,
    pub is_cached: bool,
    pub verified: bool,
    pub source_pinned: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PinnedDigest {
    Sha256(&'static str),
    GitBlobSha1(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourcePinnedFile {
    path: &'static str,
    digest: PinnedDigest,
}

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

#[derive(Debug, Error)]
pub enum ModelLifecycleError {
    #[error("model download support requires the online-model build")]
    DownloadsUnsupported,

    #[error("invalid model alias `{alias}`: {message}")]
    InvalidAlias { alias: String, message: String },

    #[error("invalid model repo `{repo_id}`: {message}")]
    InvalidRepo { repo_id: String, message: String },

    #[error("could not resolve a model cache directory")]
    CacheRootUnavailable,

    #[error("download failed for `{alias}`: {message}")]
    Download { alias: String, message: String },

    #[error("model cache at {cache_dir} is invalid: {message}")]
    CacheInvalid { cache_dir: String, message: String },

    #[error("model metadata for `{alias}` is incomplete: {message}")]
    Metadata { alias: String, message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CacheManifest {
    requested_alias: String,
    repo_id: String,
    revision: Option<String>,
    files: Vec<CachedFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CachedFile {
    path: String,
    sha256: String,
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

pub fn cache_dir_for_alias(alias: &str) -> Result<PathBuf, ModelLifecycleError> {
    let alias = resolve_model_alias(alias)?;
    cache_dir_for_resolved_alias(&alias)
}

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

pub fn download_model(
    alias: &str,
    progress: &mut impl ProgressReporter,
) -> Result<PathBuf, ModelLifecycleError> {
    #[cfg(feature = "online-model")]
    {
        return download_model_online(alias, progress);
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

    let install_result = match source_pins_for_alias(&alias) {
        Some(files) => {
            install_source_pinned_model_into_dir(&client, &alias, files, &temp_dir, progress)
        }
        None => {
            let Some(metadata) = metadata.as_ref() else {
                return Err(ModelLifecycleError::Metadata {
                    alias: alias.requested_alias.clone(),
                    message: "download metadata was not fetched for an unpinned alias".to_owned(),
                });
            };
            let files = select_files_to_download(&metadata, &alias)?;
            install_model_into_dir(&client, &alias, &files, &temp_dir, progress)
        }
    };
    if let Err(error) = install_result {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(error);
    }

    if let Err(error) = fs::rename(&temp_dir, &cache_dir) {
        if cache_dir.is_dir() && verify_cache_manifest(&cache_dir, &alias).is_ok() {
            let _ = fs::remove_dir_all(&temp_dir);
            progress.cache_hit(&cache_dir);
            return Ok(cache_dir);
        }
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(ModelLifecycleError::Download {
            alias: alias.requested_alias.clone(),
            message: format!(
                "promote verified cache {} -> {}: {error}",
                temp_dir.display(),
                cache_dir.display()
            ),
        });
    }

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
        manifest_files.push(CachedFile {
            path: artifact.relative_path,
            sha256: artifact.sha256,
            verified_from_source: artifact.verified_from_source,
        });
    }

    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = CacheManifest {
        requested_alias: alias.requested_alias.clone(),
        repo_id: alias.repo_id.clone(),
        revision: alias.revision.clone(),
        files: manifest_files,
    };
    write_manifest(temp_dir, &manifest, alias)?;
    verify_cache_manifest(temp_dir, alias)?;
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
        manifest_files.push(CachedFile {
            path: artifact.relative_path,
            sha256: artifact.sha256,
            verified_from_source: artifact.verified_from_source,
        });
    }

    manifest_files.sort_by(|left, right| left.path.cmp(&right.path));
    let manifest = CacheManifest {
        requested_alias: alias.requested_alias.clone(),
        repo_id: alias.repo_id.clone(),
        revision: alias.revision.clone(),
        files: manifest_files,
    };
    write_manifest(temp_dir, &manifest, alias)?;
    verify_cache_manifest(temp_dir, alias)?;
    Ok(())
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
                message: format!("read {url}: {error}"),
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
                    "integrity check failed for {}: expected SHA-256 {}, got {}",
                    relative_path, expected_sha256, actual_sha256
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
                message: format!("read {url}: {error}"),
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

fn download_timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

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

fn scavenge_stale_download_dirs(cache_root: &Path, cache_key: &str) {
    let Ok(entries) = fs::read_dir(cache_root) else {
        return;
    };
    let now = download_timestamp_secs();
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
        if now.saturating_sub(created_at) < STALE_DOWNLOAD_TTL.as_secs() {
            continue;
        }
        let _ = fs::remove_dir_all(entry.path());
    }
}

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
        .filter(|age| *age >= STALE_DOWNLOAD_TTL)
        .map(|age| download_timestamp_secs().saturating_sub(age.as_secs()))
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
    let _ = validated_manifest(cache_dir, alias)?;
    Ok(())
}

fn validated_manifest(
    cache_dir: &Path,
    alias: &ResolvedModelAlias,
) -> Result<CacheManifest, ModelLifecycleError> {
    let manifest =
        read_manifest(cache_dir).map_err(|message| ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message,
        })?;

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
    if manifest.files.is_empty() {
        return Err(ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message: "manifest did not record any downloaded files".to_owned(),
        });
    }

    for file in &manifest.files {
        let relative_path = normalize_relative_path(&file.path).map_err(|message| {
            ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!("manifest contains invalid path `{}`: {message}", file.path),
            }
        })?;
        let path = cache_dir.join(relative_path);
        let actual = file_sha256(&path).map_err(|message| ModelLifecycleError::CacheInvalid {
            cache_dir: cache_dir.display().to_string(),
            message,
        })?;
        if actual != file.sha256 {
            return Err(ModelLifecycleError::CacheInvalid {
                cache_dir: cache_dir.display().to_string(),
                message: format!(
                    "file hash mismatch for {}: expected {}, got {}",
                    file.path, file.sha256, actual
                ),
            });
        }
    }

    Ok(manifest)
}

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
