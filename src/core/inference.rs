//! Candle-backed text embedding and `sqlite-vec` k-NN search over the
//! `page_embeddings_vec_*` virtual tables. Two compile-time channels are
//! supported: `embedded-model` ships the airgapped BGE-small assets directly
//! in the binary; `online-model` downloads and caches a user-selected BGE
//! variant on first use. A deterministic SHA-256-based hash shim provides a
//! degraded fallback when no real model is available.
//!
//! See also: `chunking` for the page-to-chunk inputs this module embeds,
//! `search` for the hybrid composer that fuses these vector hits with FTS5.

use std::sync::{Mutex, OnceLock};

#[cfg(feature = "online-model")]
use std::fs::OpenOptions;
#[cfg(feature = "online-model")]
use std::io::ErrorKind;
#[cfg(feature = "online-model")]
use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
#[cfg(feature = "online-model")]
use candle_transformers::models::xlm_roberta::{Config as XLMRobertaConfig, XLMRobertaModel};
use rusqlite::types::ToSql;
use rusqlite::Connection;
#[cfg(feature = "online-model")]
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;
#[cfg(feature = "online-model")]
use uuid::Uuid;

use super::chunking::chunk_page;
use super::types::{InferenceError, SearchError, SearchResult};

#[cfg(feature = "online-model")]
#[derive(Debug)]
struct TempFileCleanupGuard {
    path: PathBuf,
    armed: bool,
}

#[cfg(feature = "online-model")]
impl TempFileCleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

#[cfg(feature = "online-model")]
impl Drop for TempFileCleanupGuard {
    fn drop(&mut self) {
        if !self.armed || !self.path.exists() {
            return;
        }
        if let Err(error) = std::fs::remove_file(&self.path) {
            eprintln!(
                "Warning: failed to remove temporary embedding model file {}: {error}. Run `quaid model clean --all --force` after the download exits.",
                self.path.display()
            );
        }
    }
}

const DEFAULT_MODEL_ALIAS: &str = "small";
const DEFAULT_EMBEDDING_DIMENSIONS: usize = 384;

/// Pinned SHA-256 fingerprints for the three files that make up a downloadable
/// embedding model, used for integrity verification after download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelFileHashes {
    /// Expected SHA-256 of the model's `config.json`.
    pub config_json: &'static str,
    /// Expected SHA-256 of the model's `tokenizer.json`.
    pub tokenizer_json: &'static str,
    /// Expected SHA-256 of the model's `model.safetensors` weights file.
    pub model_safetensors: &'static str,
    /// Pinned HuggingFace revision (commit SHA) used when downloading.
    /// Standard aliases always use a pinned revision for reproducibility.
    /// Custom models use `None` and fall back to `main`.
    pub revision: Option<&'static str>,
}

/// Resolved description of an embedding model: alias, HuggingFace id, output
/// dimension, and pinned hashes (when available).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelConfig {
    /// Short alias (`small`, `base`, `large`, `m3`, or `custom`).
    pub alias: String,
    /// HuggingFace repository id (`<org>/<name>`).
    pub model_id: String,
    /// Output embedding dimensionality; `0` means the dimension still needs
    /// hydration from the on-disk `config.json`.
    pub embedding_dim: usize,
    /// Pinned SHA-256 hashes; `None` for custom (unpinned) models.
    pub sha256_hashes: Option<ModelFileHashes>,
}

/// Tag indicating whether an embedding came from a real semantic model or from
/// the deterministic hash-based fallback shim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingEvidenceKind {
    /// Embedding was produced by a real Candle BERT or XLM-RoBERTa model.
    Semantic,
    /// Embedding was produced by the deterministic SHA-256 fallback shim.
    HashShim,
}

impl ModelConfig {
    /// Returns the `sqlite-vec` virtual-table name that stores embeddings of
    /// this model's dimensionality.
    pub fn vec_table(&self) -> String {
        format!("page_embeddings_vec_{}", self.embedding_dim)
    }

    /// Returns the HuggingFace `<org>/<name>` id used as the persisted model name.
    pub fn embedding_model_name(&self) -> &str {
        &self.model_id
    }

    /// Returns a short, user-facing label for the model: the alias for
    /// standard models, or the full model id for custom ones.
    pub fn model_hint(&self) -> &str {
        if self.alias == "custom" {
            &self.model_id
        } else {
            &self.alias
        }
    }

    /// Returns `true` when this is the default BGE-small model used by the
    /// airgapped (embedded-model) build channel.
    pub fn is_small(&self) -> bool {
        self.alias == "small" || self.model_id == "BAAI/bge-small-en-v1.5"
    }

    /// Returns `true` when the embedding dimension still needs to be read out
    /// of the model's `config.json` (only possible for custom models).
    pub fn needs_dimension_hydration(&self) -> bool {
        self.embedding_dim == 0
    }
}

const SMALL_HASHES: ModelFileHashes = ModelFileHashes {
    config_json: "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    tokenizer_json: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    model_safetensors: "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    revision: Some("5c38ec7c405ec4b44b94cc5a9bb96e735b38267a"),
};

const BASE_HASHES: ModelFileHashes = ModelFileHashes {
    config_json: "bc00af31a4a31b74040d73370aa83b62da34c90b75eb77bfa7db039d90abd591",
    tokenizer_json: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    model_safetensors: "c7c1988aae201f80cf91a5dbbd5866409503b89dcaba877ca6dba7dd0a5167d7",
    revision: Some("a5beb1e3e68b9ab74eb54cfd186867f64f240e1a"),
};

const LARGE_HASHES: ModelFileHashes = ModelFileHashes {
    config_json: "446712fac367857b4b1302762fe1cd7bfa8b3c4b77b4dc5d77c4025407660896",
    tokenizer_json: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    model_safetensors: "45e1954914e29bd74080e6c1510165274ff5279421c89f76c418878732f64ae7",
    revision: Some("d9e9d73f56c5e5851e28a1bcbe3b1c36e3d28d4c"),
};

const M3_HASHES: ModelFileHashes = ModelFileHashes {
    config_json: "26159e7ad065073448460117eb24b7a4572f6f4e78eadff65dc0a11c052449fa",
    tokenizer_json: "21106b6d7dab2952c1d496fb21d5dc9db75c28ed361a05f5020bbba27810dd08",
    model_safetensors: "993b2248881724788dcab8c644a91dfd63584b6e5604ff2037cb5541e1e38e7e",
    revision: Some("babcf60cae0a1f438d7ade582983571a6b46523f"),
};

/// Returns the [`ModelConfig`] for the default model (`BAAI/bge-small-en-v1.5`).
pub fn default_model() -> ModelConfig {
    resolve_model(DEFAULT_MODEL_ALIAS)
}

/// Resolves a user-supplied model alias or HuggingFace id into a
/// [`ModelConfig`], falling back to a custom-model record (with no pinned
/// hashes) for unknown inputs.
pub fn resolve_model(input: &str) -> ModelConfig {
    let trimmed = input.trim();
    let normalized = trimmed.to_ascii_lowercase();

    match normalized.as_str() {
        "" | DEFAULT_MODEL_ALIAS => ModelConfig {
            alias: "small".to_owned(),
            model_id: "BAAI/bge-small-en-v1.5".to_owned(),
            embedding_dim: 384,
            sha256_hashes: Some(SMALL_HASHES),
        },
        "base" => ModelConfig {
            alias: "base".to_owned(),
            model_id: "BAAI/bge-base-en-v1.5".to_owned(),
            embedding_dim: 768,
            sha256_hashes: Some(BASE_HASHES),
        },
        "large" => ModelConfig {
            alias: "large".to_owned(),
            model_id: "BAAI/bge-large-en-v1.5".to_owned(),
            embedding_dim: 1024,
            sha256_hashes: Some(LARGE_HASHES),
        },
        "m3" => ModelConfig {
            alias: "m3".to_owned(),
            model_id: "BAAI/bge-m3".to_owned(),
            embedding_dim: 1024,
            sha256_hashes: Some(M3_HASHES),
        },
        "baai/bge-small-en-v1.5" => ModelConfig {
            alias: "small".to_owned(),
            model_id: "BAAI/bge-small-en-v1.5".to_owned(),
            embedding_dim: 384,
            sha256_hashes: Some(SMALL_HASHES),
        },
        "baai/bge-base-en-v1.5" => ModelConfig {
            alias: "base".to_owned(),
            model_id: "BAAI/bge-base-en-v1.5".to_owned(),
            embedding_dim: 768,
            sha256_hashes: Some(BASE_HASHES),
        },
        "baai/bge-large-en-v1.5" => ModelConfig {
            alias: "large".to_owned(),
            model_id: "BAAI/bge-large-en-v1.5".to_owned(),
            embedding_dim: 1024,
            sha256_hashes: Some(LARGE_HASHES),
        },
        "baai/bge-m3" => ModelConfig {
            alias: "m3".to_owned(),
            model_id: "BAAI/bge-m3".to_owned(),
            embedding_dim: 1024,
            sha256_hashes: Some(M3_HASHES),
        },
        _ => {
            eprintln!(
                "Warning: unpinned custom embedding model `{trimmed}`; skipping SHA-256 verification."
            );
            ModelConfig {
                alias: "custom".to_owned(),
                model_id: trimmed.to_owned(),
                embedding_dim: 0,
                sha256_hashes: None,
            }
        }
    }
}

/// Returns the built-in embedding model aliases that use pinned file hashes.
pub fn known_embedding_models() -> Vec<ModelConfig> {
    ["small", "base", "large", "m3"]
        .into_iter()
        .map(resolve_model)
        .collect()
}

/// Resolves a selector only when it names one of Quaid's pinned embedding
/// model aliases or repository ids, avoiding custom-model warning output.
pub fn resolve_known_embedding_model(input: &str) -> Option<ModelConfig> {
    let normalized = input.trim().to_ascii_lowercase();
    let alias = match normalized.as_str() {
        "" | "small" | "baai/bge-small-en-v1.5" => "small",
        "base" | "baai/bge-base-en-v1.5" => "base",
        "large" | "baai/bge-large-en-v1.5" => "large",
        "m3" | "baai/bge-m3" => "m3",
        _ => return None,
    };
    Some(resolve_model(alias))
}

/// Resolves an optional user-supplied model selector, defaulting to the
/// embedded model and coercing the result to what the current build channel
/// can actually load.
pub fn resolve_requested_model(input: Option<&str>) -> ModelConfig {
    let requested = resolve_model(input.unwrap_or(DEFAULT_MODEL_ALIAS));
    coerce_model_for_build(&requested)
}

/// Returns `requested` unchanged on the `online-model` build, but on the
/// airgapped `embedded-model` build silently falls back to the embedded
/// BGE-small model (with a warning) if anything else was asked for.
pub fn coerce_model_for_build(requested: &ModelConfig) -> ModelConfig {
    #[cfg(feature = "embedded-model")]
    {
        if !requested.is_small() {
            eprintln!(
                "Warning: --model / QUAID_MODEL is only configurable in the online-model build; continuing with BAAI/bge-small-en-v1.5."
            );
            return default_model();
        }
    }

    requested.clone()
}

/// Fills in a custom model's missing embedding dimension by reading
/// `hidden_size` from its `config.json`; standard models pass through
/// unchanged. Requires the `online-model` feature for custom models.
pub fn hydrate_model_config(model: &ModelConfig) -> Result<ModelConfig, String> {
    if !model.needs_dimension_hydration() {
        return Ok(model.clone());
    }

    #[cfg(feature = "online-model")]
    {
        let (config_path, _, _) = download_model_files(model)?;
        let embedding_dim = read_embedding_dim_from_config(&config_path)?;
        let mut hydrated = model.clone();
        hydrated.embedding_dim = embedding_dim;
        Ok(hydrated)
    }

    #[cfg(not(feature = "online-model"))]
    {
        Err(format!(
            "custom model {} requires the online-model build to resolve dimensions",
            model.model_id
        ))
    }
}

struct ModelRuntime {
    configured: ModelConfig,
    loaded: Option<EmbeddingModel>,
}

impl Default for ModelRuntime {
    fn default() -> Self {
        Self {
            configured: default_model(),
            loaded: None,
        }
    }
}

static MODEL_RUNTIME: OnceLock<Mutex<ModelRuntime>> = OnceLock::new();

fn model_runtime() -> &'static Mutex<ModelRuntime> {
    MODEL_RUNTIME.get_or_init(|| Mutex::new(ModelRuntime::default()))
}

/// Sets the process-wide embedding model. Subsequent calls to [`embed`] and
/// [`search_vec`] will load the new model on first use; the previously loaded
/// model is dropped if the configuration changes.
pub fn configure_runtime_model(model: ModelConfig) {
    let mut runtime = model_runtime().lock().unwrap_or_else(|e| e.into_inner());
    if runtime.configured != model {
        runtime.configured = model;
        runtime.loaded = None;
    }
}

/// Compatibility alias for [`configure_runtime_model`].
pub fn set_model_config(model: ModelConfig) {
    configure_runtime_model(model);
}

fn runtime_model_config() -> ModelConfig {
    model_runtime()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .configured
        .clone()
}

/// Loaded embedding backend (real Candle model or hash-shim fallback) plus the
/// [`ModelConfig`] it was instantiated from. Held in a process-wide
/// [`OnceLock`]-guarded mutex; callers should reach for [`embed`] and
/// [`search_vec`] rather than constructing this directly.
pub struct EmbeddingModel {
    config: ModelConfig,
    backend: EmbeddingBackend,
}

enum EmbeddingBackend {
    CandleBert {
        model: Box<BertModel>,
        tokenizer: Box<Tokenizer>,
        device: Device,
    },
    #[cfg(feature = "online-model")]
    CandleXlmRoberta {
        model: Box<XLMRobertaModel>,
        tokenizer: Box<Tokenizer>,
        device: Device,
        max_len: usize,
    },
    HashShim,
}

impl std::fmt::Debug for EmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            EmbeddingBackend::CandleBert { .. } => f
                .debug_struct("EmbeddingModel")
                .field("backend", &format!("Candle({})", self.config.model_id))
                .finish(),
            #[cfg(feature = "online-model")]
            EmbeddingBackend::CandleXlmRoberta { .. } => f
                .debug_struct("EmbeddingModel")
                .field("backend", &format!("Candle({})", self.config.model_id))
                .finish(),
            EmbeddingBackend::HashShim => f
                .debug_struct("EmbeddingModel")
                .field("backend", &"HashShim")
                .field("model_id", &self.config.model_id)
                .field("embedding_dim", &self.config.embedding_dim)
                .finish(),
        }
    }
}

impl EmbeddingModel {
    fn new(config: ModelConfig) -> Self {
        let hydrated = hydrate_model_config(&config).unwrap_or(config);

        match Self::try_load_candle(&hydrated) {
            Ok(backend) => Self {
                config: hydrated,
                backend,
            },
            Err(err) => {
                eprintln!(
                    "Warning: embedding model {} not available ({err}), using hash-based embeddings. Rebuild the airgapped channel with `cargo build --release` or the online channel with `cargo build --release --no-default-features --features bundled,online-model`, then run `quaid embed --all`.",
                    hydrated.model_id
                );
                Self {
                    config: hydrated,
                    backend: EmbeddingBackend::HashShim,
                }
            }
        }
    }

    fn try_load_candle(config: &ModelConfig) -> Result<EmbeddingBackend, String> {
        #[cfg(all(feature = "online-model", feature = "test-harness"))]
        {
            load_online_backend(config)
        }

        // When both release channels are enabled outside the test harness,
        // prefer embedded so validation builds keep release-profile behavior.
        #[cfg(all(
            feature = "embedded-model",
            not(all(feature = "online-model", feature = "test-harness"))
        ))]
        {
            load_embedded_backend(config)
        }

        #[cfg(all(
            not(feature = "embedded-model"),
            feature = "online-model",
            not(feature = "test-harness")
        ))]
        {
            load_online_backend(config)
        }

        #[cfg(not(any(feature = "embedded-model", feature = "online-model")))]
        {
            let _ = config;
            Err("no model channel enabled".to_owned())
        }
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, InferenceError> {
        match &self.backend {
            EmbeddingBackend::CandleBert {
                model,
                tokenizer,
                device,
            } => embed_candle(text, model, tokenizer, device),
            #[cfg(feature = "online-model")]
            EmbeddingBackend::CandleXlmRoberta {
                model,
                tokenizer,
                device,
                max_len,
            } => embed_candle_xlm_roberta(text, model, tokenizer, device, *max_len),
            EmbeddingBackend::HashShim => embed_hash_shim(text, self.config.embedding_dim),
        }
    }

    fn evidence_kind(&self) -> EmbeddingEvidenceKind {
        match self.backend {
            EmbeddingBackend::HashShim => EmbeddingEvidenceKind::HashShim,
            _ => EmbeddingEvidenceKind::Semantic,
        }
    }
}

#[cfg(all(
    feature = "embedded-model",
    not(all(feature = "online-model", feature = "test-harness"))
))]
fn load_embedded_backend(config: &ModelConfig) -> Result<EmbeddingBackend, String> {
    if !config.is_small() {
        return Err(format!(
            "embedded-model build only includes BAAI/bge-small-en-v1.5, requested {}",
            config.model_id
        ));
    }

    let config: BertConfig =
        serde_json::from_slice(include_bytes!(env!("QUAID_EMBEDDED_CONFIG_PATH")))
            .map_err(|e| format!("parse embedded config.json: {e}"))?;
    let tokenizer = Tokenizer::from_bytes(include_bytes!(env!("QUAID_EMBEDDED_TOKENIZER_PATH")))
        .map_err(|e| format!("load embedded tokenizer: {e}"))?;
    let device = Device::Cpu;
    let vb = VarBuilder::from_slice_safetensors(
        include_bytes!(env!("QUAID_EMBEDDED_MODEL_PATH")),
        DType::F32,
        &device,
    )
    .map_err(|e| format!("load embedded model weights: {e}"))?;
    let model = BertModel::load(vb, &config).map_err(|e| format!("build BERT model: {e}"))?;

    Ok(EmbeddingBackend::CandleBert {
        model: Box::new(model),
        tokenizer: Box::new(tokenizer),
        device,
    })
}

#[cfg(feature = "online-model")]
fn load_online_backend(config: &ModelConfig) -> Result<EmbeddingBackend, String> {
    // In tests, set QUAID_FORCE_HASH_SHIM=1 to skip the 300s download
    // attempt and use the deterministic hash-based shim instead.  This keeps
    // tests fast and avoids real network calls.
    if std::env::var("QUAID_FORCE_HASH_SHIM").as_deref() == Ok("1") {
        return Err("QUAID_FORCE_HASH_SHIM=1: skipping model download in test mode".to_owned());
    }
    let (config_path, tokenizer_path, model_path) = download_model_files(config)?;
    let model_type = read_model_type_from_config(&config_path)?;
    let config_text =
        std::fs::read_to_string(&config_path).map_err(|e| format!("read config.json: {e}"))?;
    let tokenizer =
        Tokenizer::from_file(&tokenizer_path).map_err(|e| format!("load tokenizer: {e}"))?;
    let device = Device::Cpu;

    match model_type.as_str() {
        "bert" => {
            let config: BertConfig = serde_json::from_str(&config_text)
                .map_err(|e| format!("parse config.json: {e}"))?;
            #[expect(
                unsafe_code,
                reason = "candle's VarBuilder::from_mmaped_safetensors mmaps tensor data; safety hinges on the file not being mutated for the lifetime of the VarBuilder, which we uphold by reading from immutable on-disk model weights"
            )]
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                    .map_err(|e| format!("load model weights: {e}"))?
            };
            let model =
                BertModel::load(vb, &config).map_err(|e| format!("build BERT model: {e}"))?;

            Ok(EmbeddingBackend::CandleBert {
                model: Box::new(model),
                tokenizer: Box::new(tokenizer),
                device,
            })
        }
        "xlm-roberta" => {
            let max_len = read_max_position_embeddings_from_config(&config_path)?;
            let config: XLMRobertaConfig = serde_json::from_str(&config_text)
                .map_err(|e| format!("parse config.json: {e}"))?;
            #[expect(
                unsafe_code,
                reason = "candle's VarBuilder::from_mmaped_safetensors mmaps tensor data; safety hinges on the file not being mutated for the lifetime of the VarBuilder, which we uphold by reading from immutable on-disk model weights"
            )]
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
                    .map_err(|e| format!("load model weights: {e}"))?
            };
            let model =
                XLMRobertaModel::new(&config, vb).map_err(|e| format!("build XLM-R model: {e}"))?;

            Ok(EmbeddingBackend::CandleXlmRoberta {
                model: Box::new(model),
                tokenizer: Box::new(tokenizer),
                device,
                max_len,
            })
        }
        _ => Err(format!(
            "model architecture {model_type} is not supported by the current Candle loader"
        )),
    }
}

fn embed_candle(
    text: &str,
    model: &BertModel,
    tokenizer: &Tokenizer,
    device: &Device,
) -> Result<Vec<f32>, InferenceError> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| InferenceError::Internal {
            message: format!("tokenizer: {e}"),
        })?;

    let max_len = 512;
    let ids: &[u32] = &encoding.get_ids()[..encoding.get_ids().len().min(max_len)];
    let mask: &[u32] =
        &encoding.get_attention_mask()[..encoding.get_attention_mask().len().min(max_len)];

    let input_ids = Tensor::new(ids, device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| InferenceError::Internal {
            message: format!("input_ids tensor: {e}"),
        })?;

    let token_type_ids = input_ids
        .zeros_like()
        .map_err(|e| InferenceError::Internal {
            message: format!("token_type_ids: {e}"),
        })?;

    let attention_mask = Tensor::new(mask, device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| InferenceError::Internal {
            message: format!("attention_mask tensor: {e}"),
        })?;

    let output = model
        .forward(&input_ids, &token_type_ids, Some(&attention_mask))
        .map_err(|e| InferenceError::Internal {
            message: format!("BERT forward: {e}"),
        })?;

    mean_pool_and_normalize(output, attention_mask)
}

#[cfg(feature = "online-model")]
fn embed_candle_xlm_roberta(
    text: &str,
    model: &XLMRobertaModel,
    tokenizer: &Tokenizer,
    device: &Device,
    max_len: usize,
) -> Result<Vec<f32>, InferenceError> {
    let encoding = tokenizer
        .encode(text, true)
        .map_err(|e| InferenceError::Internal {
            message: format!("tokenizer: {e}"),
        })?;

    let ids: &[u32] = &encoding.get_ids()[..encoding.get_ids().len().min(max_len)];
    let mask: &[u32] =
        &encoding.get_attention_mask()[..encoding.get_attention_mask().len().min(max_len)];

    let input_ids = Tensor::new(ids, device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| InferenceError::Internal {
            message: format!("input_ids tensor: {e}"),
        })?;

    let attention_mask = Tensor::new(mask, device)
        .and_then(|t| t.unsqueeze(0))
        .map_err(|e| InferenceError::Internal {
            message: format!("attention_mask tensor: {e}"),
        })?;

    let token_type_ids = input_ids
        .zeros_like()
        .map_err(|e| InferenceError::Internal {
            message: format!("token_type_ids: {e}"),
        })?;

    let output = model
        .forward(
            &input_ids,
            &attention_mask,
            &token_type_ids,
            None,
            None,
            None,
        )
        .map_err(|e| InferenceError::Internal {
            message: format!("XLM-R forward: {e}"),
        })?;

    mean_pool_and_normalize(output, attention_mask)
}

fn mean_pool_and_normalize(
    output: Tensor,
    attention_mask: Tensor,
) -> Result<Vec<f32>, InferenceError> {
    let mask_f32 = attention_mask
        .unsqueeze(2)
        .and_then(|t| t.to_dtype(DType::F32))
        .map_err(|e| InferenceError::Internal {
            message: format!("mask expand: {e}"),
        })?;

    let mask_broadcast =
        mask_f32
            .broadcast_as(output.shape())
            .map_err(|e| InferenceError::Internal {
                message: format!("mask broadcast: {e}"),
            })?;

    let masked = output
        .mul(&mask_broadcast)
        .map_err(|e| InferenceError::Internal {
            message: format!("mask mul: {e}"),
        })?;

    let sum = masked.sum(1).map_err(|e| InferenceError::Internal {
        message: format!("sum: {e}"),
    })?;

    let count = mask_f32.sum(1).map_err(|e| InferenceError::Internal {
        message: format!("count: {e}"),
    })?;

    let count_broadcast =
        count
            .broadcast_as(sum.shape())
            .map_err(|e| InferenceError::Internal {
                message: format!("count broadcast: {e}"),
            })?;

    let mean = sum
        .div(&count_broadcast)
        .map_err(|e| InferenceError::Internal {
            message: format!("mean: {e}"),
        })?;

    let norm = mean
        .sqr()
        .and_then(|t| t.sum_keepdim(1))
        .and_then(|t| t.sqrt())
        .map_err(|e| InferenceError::Internal {
            message: format!("norm: {e}"),
        })?;

    let norm_broadcast = norm
        .broadcast_as(mean.shape())
        .map_err(|e| InferenceError::Internal {
            message: format!("norm broadcast: {e}"),
        })?;

    let normalized = mean
        .div(&norm_broadcast)
        .map_err(|e| InferenceError::Internal {
            message: format!("normalize: {e}"),
        })?;

    normalized
        .squeeze(0)
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(|e| InferenceError::Internal {
            message: format!("to_vec: {e}"),
        })
}

#[cfg(feature = "online-model")]
fn download_model_files(model: &ModelConfig) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    validate_model_id(&model.model_id)?;
    let cache_dir = model_cache_dir(model)?;

    if let Some(paths) = existing_model_paths(&cache_dir) {
        verify_cached_model_integrity(model, &cache_dir)?;
        return Ok(paths);
    }

    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("create model cache {}: {e}", cache_dir.display()))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent("quaid-runtime/0.9.10")
        .build()
        .map_err(|e| format!("build download client: {e}"))?;

    if model.sha256_hashes.is_none() {
        eprintln!(
            "Warning: custom model {} has no pinned SHA-256 hashes; skipping integrity verification.",
            model.model_id
        );
    }

    for file_name in ["config.json", "tokenizer.json", "model.safetensors"] {
        download_model_file(&client, model, file_name, &cache_dir)?;
    }

    existing_model_paths(&cache_dir).ok_or_else(|| {
        format!(
            "model files missing after download in {}",
            cache_dir.display()
        )
    })
}

#[cfg(feature = "online-model")]
fn create_temp_download_file(
    cache_dir: &Path,
    file_name: &str,
) -> Result<(PathBuf, std::fs::File), String> {
    for _ in 0..10 {
        let temp_path = cache_dir.join(format!("{file_name}.download-{}", Uuid::new_v4()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(err) if err.kind() == ErrorKind::AlreadyExists => continue,
            Err(err) => {
                return Err(format!("create {}: {err}", temp_path.display()));
            }
        }
    }
    Err(format!(
        "unable to create temp download file for {file_name} in {}",
        cache_dir.display()
    ))
}

#[cfg(feature = "online-model")]
fn download_model_file(
    client: &reqwest::blocking::Client,
    model: &ModelConfig,
    file_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    let validated_model_id = validate_model_id(&model.model_id)?;
    let destination = cache_dir.join(file_name);
    let (temp_destination, file) = create_temp_download_file(cache_dir, file_name)?;
    let mut temp_guard = TempFileCleanupGuard::new(temp_destination);
    let mut file = file;
    let base_url = huggingface_base_url();
    // Use pinned revision for standard aliases; fall back to `main` only for
    // custom/unpinned models so upstream changes never silently break SHA checks.
    let revision = model
        .sha256_hashes
        .and_then(|h| h.revision)
        .unwrap_or("main");
    let url = format!(
        "{}/{}/resolve/{}/{}",
        base_url.trim_end_matches('/'),
        validated_model_id,
        revision,
        file_name
    );

    let mut response = client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("download {url}: {e}"))?;

    std::io::copy(&mut response, &mut file)
        .map_err(|e| format!("write {}: {e}", temp_guard.path().display()))?;
    file.sync_all()
        .map_err(|e| format!("flush {}: {e}", temp_guard.path().display()))?;

    if let Some(expected_hash) = expected_hash_for_file(model, file_name) {
        verify_file_sha256(temp_guard.path(), expected_hash).map_err(|error| {
            format!(
                "{error}; run `quaid model clean {} --force` and retry the operation",
                model.alias
            )
        })?;
    }

    drop(file);
    if let Err(err) = std::fs::rename(temp_guard.path(), &destination) {
        if destination.exists() {
            if let Some(expected_hash) = expected_hash_for_file(model, file_name) {
                verify_file_sha256(&destination, expected_hash)?;
            }
            return Ok(());
        }
        return Err(format!(
            "rename {}: {err}; temporary file {} will be removed",
            destination.display(),
            temp_guard.path().display()
        ));
    }
    temp_guard.disarm();

    Ok(())
}

#[cfg(feature = "online-model")]
fn expected_hash_for_file(model: &ModelConfig, file_name: &str) -> Option<&'static str> {
    let hashes = model.sha256_hashes?;
    match file_name {
        "config.json" => Some(hashes.config_json),
        "tokenizer.json" => Some(hashes.tokenizer_json),
        "model.safetensors" => Some(hashes.model_safetensors),
        _ => None,
    }
}

#[cfg(feature = "online-model")]
fn verify_file_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("open {} for hash: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("read {} for hash: {e}", path.display()))?;
    let actual = format!("{:x}", hasher.finalize());

    if actual != expected {
        return Err(format!(
            "SHA-256 mismatch for {}: expected {expected}, got {actual}",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(feature = "online-model")]
fn verify_cached_model_integrity(model: &ModelConfig, cache_dir: &Path) -> Result<(), String> {
    if let Some(hashes) = model.sha256_hashes {
        for (file_name, expected_hash) in [
            ("config.json", hashes.config_json),
            ("tokenizer.json", hashes.tokenizer_json),
            ("model.safetensors", hashes.model_safetensors),
        ] {
            let path = cache_dir.join(file_name);
            verify_file_sha256(&path, expected_hash).map_err(|e| {
                format!(
                    "cached model integrity check failed — delete {} and retry: {e}",
                    cache_dir.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(feature = "online-model")]
fn huggingface_base_url() -> String {
    std::env::var("QUAID_HF_BASE_URL").unwrap_or_else(|_| "https://huggingface.co".to_owned())
}

#[cfg(feature = "online-model")]
fn model_cache_dir(model: &ModelConfig) -> Result<PathBuf, String> {
    if let Ok(cache_root) = std::env::var("QUAID_MODEL_CACHE_DIR") {
        return Ok(PathBuf::from(cache_root).join(cache_dir_name(model)));
    }

    dirs::home_dir()
        .map(|home| {
            home.join(".quaid")
                .join("models")
                .join(cache_dir_name(model))
        })
        .ok_or_else(|| "could not resolve home directory for model cache".to_owned())
}

#[cfg(feature = "online-model")]
fn cache_dir_name(model: &ModelConfig) -> String {
    cache_dir_name_from_model_id(&model.model_id)
}

#[cfg(feature = "online-model")]
fn existing_model_paths(cache_dir: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let config = cache_dir.join("config.json");
    let tokenizer = cache_dir.join("tokenizer.json");
    let model = cache_dir.join("model.safetensors");

    (config.is_file() && tokenizer.is_file() && model.is_file())
        .then_some((config, tokenizer, model))
}

/// Returns the cache directory used for a resolved online embedding model.
#[cfg(feature = "online-model")]
pub fn embedding_model_cache_dir(model: &ModelConfig) -> Result<PathBuf, String> {
    model_cache_dir(model)
}

/// Returns the sanitized cache key used for a resolved online embedding model.
#[cfg(feature = "online-model")]
pub fn embedding_model_cache_key(model: &ModelConfig) -> String {
    cache_dir_name(model)
}

/// Returns the required file names for an online embedding model cache.
#[cfg(feature = "online-model")]
pub fn embedding_required_files() -> &'static [&'static str] {
    &["config.json", "tokenizer.json", "model.safetensors"]
}

/// Validates the required files for an online embedding model cache, with
/// optional SHA-256 verification for pinned models.
#[cfg(feature = "online-model")]
pub fn verify_embedding_model_cache(
    model: &ModelConfig,
    cache_dir: &Path,
    verify_hashes: bool,
) -> Result<(), String> {
    for file_name in embedding_required_files() {
        let path = cache_dir.join(file_name);
        if !path.is_file() {
            return Err(format!("missing required file {}", path.display()));
        }
    }
    if verify_hashes {
        verify_cached_model_integrity(model, cache_dir)?;
    }
    Ok(())
}

#[cfg(feature = "online-model")]
fn read_embedding_dim_from_config(path: &Path) -> Result<usize, String> {
    let config_json = read_config_json(path)?;

    config_json["hidden_size"]
        .as_u64()
        .map(|value| value as usize)
        .ok_or_else(|| format!("hidden_size missing in {}", path.display()))
}

#[cfg(feature = "online-model")]
fn read_model_type_from_config(path: &Path) -> Result<String, String> {
    let config_json = read_config_json(path)?;

    config_json["model_type"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("model_type missing in {}", path.display()))
}

#[cfg(feature = "online-model")]
fn read_max_position_embeddings_from_config(path: &Path) -> Result<usize, String> {
    let config_json = read_config_json(path)?;

    config_json["max_position_embeddings"]
        .as_u64()
        .map(|value| value as usize)
        .ok_or_else(|| format!("max_position_embeddings missing in {}", path.display()))
}

#[cfg(feature = "online-model")]
fn read_config_json(path: &Path) -> Result<Value, String> {
    let config_text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&config_text).map_err(|e| format!("parse {}: {e}", path.display()))
}

#[cfg(feature = "online-model")]
fn validate_model_id(model_id: &str) -> Result<&str, String> {
    if model_id.trim() != model_id || model_id.is_empty() {
        return Err(format!(
            "invalid model id `{model_id}`: expected <org>/<name> without surrounding whitespace"
        ));
    }

    if model_id
        .chars()
        .any(|ch| matches!(ch, ' ' | '\t' | '\n' | '\r' | '#' | '?' | '\\'))
    {
        return Err(format!(
            "invalid model id `{model_id}`: spaces, '\\\\', '#', and '?' are not allowed"
        ));
    }

    let mut segments = model_id.split('/');
    let Some(namespace) = segments.next() else {
        return Err(format!("invalid model id `{model_id}`"));
    };
    let Some(name) = segments.next() else {
        return Err(format!(
            "invalid model id `{model_id}`: expected exactly one '/' separator"
        ));
    };

    if segments.next().is_some() {
        return Err(format!(
            "invalid model id `{model_id}`: expected exactly one '/' separator"
        ));
    }

    if !is_valid_model_segment(namespace) || !is_valid_model_segment(name) {
        return Err(format!(
            "invalid model id `{model_id}`: each path segment must be non-empty and cannot be '.' or '..'"
        ));
    }

    Ok(model_id)
}

#[cfg(feature = "online-model")]
fn is_valid_model_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && segment
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
}

#[cfg(feature = "online-model")]
fn cache_dir_name_from_model_id(model_id: &str) -> String {
    let mut segments = model_id.split('/');
    let Some(namespace) = segments.next() else {
        return hashed_cache_dir_name(model_id);
    };
    let Some(name) = segments.next() else {
        return hashed_cache_dir_name(model_id);
    };

    if segments.next().is_some() {
        return hashed_cache_dir_name(model_id);
    }

    let namespace = sanitize_cache_segment(namespace);
    let name = sanitize_cache_segment(name);

    match (namespace, name) {
        (Some(namespace), Some(name)) => format!("{namespace}--{name}"),
        _ => hashed_cache_dir_name(model_id),
    }
}

#[cfg(feature = "online-model")]
fn sanitize_cache_segment(segment: &str) -> Option<String> {
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.chars().any(|ch| ch == '/' || ch == '\\')
    {
        return None;
    }

    let sanitized: String = segment
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect();

    if sanitized.is_empty() || sanitized.chars().all(|ch| ch == '.') {
        None
    } else {
        Some(sanitized)
    }
}

#[cfg(feature = "online-model")]
fn hashed_cache_dir_name(model_id: &str) -> String {
    let digest = Sha256::digest(model_id.as_bytes());
    format!("custom-{digest:x}")
}

fn embed_hash_shim(text: &str, embedding_dim: usize) -> Result<Vec<f32>, InferenceError> {
    let embedding_dim = if embedding_dim == 0 {
        DEFAULT_EMBEDDING_DIMENSIONS
    } else {
        embedding_dim
    };
    let mut embedding = vec![0.0; embedding_dim];

    for (token_index, token) in text.split_whitespace().enumerate() {
        accumulate_token_hash(token, token_index, &mut embedding);
    }

    if embedding.iter().all(|value| *value == 0.0) {
        accumulate_token_hash(text, 0, &mut embedding);
    }

    normalize(&mut embedding)?;
    Ok(embedding)
}

fn accumulate_token_hash(token: &str, token_index: usize, embedding: &mut [f32]) {
    let chunk_count = embedding.len().div_ceil(32);
    for chunk_index in 0..chunk_count {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher.update((token_index as u64).to_le_bytes());
        hasher.update((chunk_index as u64).to_le_bytes());
        let digest = hasher.finalize();
        let start = chunk_index * 32;

        for (offset, byte) in digest.iter().enumerate() {
            let target = start + offset;
            if target >= embedding.len() {
                break;
            }

            let centered = (*byte as f32 / 127.5) - 1.0;
            embedding[target] += centered;
        }
    }
}

/// Lazily loads the configured embedding model into the process-wide runtime
/// slot if it has not been loaded yet (or if the configured model changed
/// since the last load).
pub fn ensure_model() {
    let configured = runtime_model_config();

    // Check under lock whether a reload is needed, then release before doing
    // the expensive download/mmap so concurrent callers (e.g. `quaid serve`)
    // are not blocked for the full model-load duration.
    let needs_reload = {
        let runtime = model_runtime().lock().unwrap_or_else(|e| e.into_inner());
        runtime
            .loaded
            .as_ref()
            .map(|loaded| loaded.config != configured)
            .unwrap_or(true)
    };

    if needs_reload {
        let new_model = EmbeddingModel::new(configured.clone());
        let mut runtime = model_runtime().lock().unwrap_or_else(|e| e.into_inner());
        // Re-check in case another thread loaded the same model while we were
        // building it — avoid an unnecessary double-install.
        let still_needs_reload = runtime
            .loaded
            .as_ref()
            .map(|loaded| loaded.config != configured)
            .unwrap_or(true);
        if still_needs_reload {
            runtime.loaded = Some(new_model);
        }
    }
}

/// Embeds `text` into a normalized vector with the currently configured model,
/// loading the model on first use. Returns [`InferenceError::EmptyInput`] for
/// blank input.
pub fn embed(text: &str) -> Result<Vec<f32>, InferenceError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(InferenceError::EmptyInput);
    }

    ensure_model();
    let runtime = model_runtime().lock().unwrap_or_else(|e| e.into_inner());
    // Use `ok_or` rather than `expect` so a race between configure_runtime_model
    // (which sets loaded = None) and embed() returns an error instead of a panic.
    runtime
        .loaded
        .as_ref()
        .ok_or_else(|| InferenceError::Internal {
            message: "embedding model is not loaded; call configure_runtime_model first".to_owned(),
        })?
        .embed(trimmed)
}

/// Reports whether the currently loaded model is the real semantic backend or
/// the deterministic hash-based shim. Used by callers that want to label or
/// downrank evidence produced by the fallback.
pub fn embedding_evidence_kind() -> Result<EmbeddingEvidenceKind, InferenceError> {
    ensure_model();
    let runtime = model_runtime().lock().unwrap_or_else(|e| e.into_inner());
    Ok(runtime
        .loaded
        .as_ref()
        .ok_or_else(|| InferenceError::Internal {
            message: "embedding model is not loaded; call configure_runtime_model first".to_owned(),
        })?
        .evidence_kind())
}

/// Embeds `query` and returns the top `k` semantically nearest pages (by
/// cosine distance) optionally filtered by wing and collection.
pub fn search_vec(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_with_namespace(query, k, wing_filter, collection_filter, None, conn)
}

/// Namespace-aware variant of [`search_vec`].
pub fn search_vec_with_namespace(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_with_namespace_filtered(
        query,
        k,
        wing_filter,
        collection_filter,
        namespace_filter,
        false,
        conn,
    )
}

/// Namespace-aware variant of [`search_vec`] that also exposes the
/// `include_superseded` toggle for callers that want to inspect history.
pub fn search_vec_with_namespace_filtered(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_internal(
        query,
        k,
        wing_filter,
        collection_filter,
        namespace_filter,
        include_superseded,
        conn,
        false,
    )
}

/// Canonical-slug variant of [`search_vec`]: returns slugs in
/// `<collection>::<slug>` form so cross-collection results can be disambiguated.
pub fn search_vec_canonical(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_canonical_with_namespace(query, k, wing_filter, collection_filter, None, conn)
}

/// Namespace-aware canonical-slug variant of [`search_vec`].
pub fn search_vec_canonical_with_namespace(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_canonical_with_namespace_filtered(
        query,
        k,
        wing_filter,
        collection_filter,
        namespace_filter,
        false,
        conn,
    )
}

/// Namespace-aware canonical-slug variant of [`search_vec`] that also exposes
/// the `include_superseded` toggle.
pub fn search_vec_canonical_with_namespace_filtered(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
) -> Result<Vec<SearchResult>, SearchError> {
    search_vec_internal(
        query,
        k,
        wing_filter,
        collection_filter,
        namespace_filter,
        include_superseded,
        conn,
        true,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "internal vector-search dispatcher binds the full search context (query, k, wing, collection, namespace, superseded flag, conn, canonical flag); the public wrappers are the right boundary for grouping"
)]
fn search_vec_internal(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&str>,
    include_superseded: bool,
    conn: &Connection,
    canonical_slug: bool,
) -> Result<Vec<SearchResult>, SearchError> {
    if query.trim().is_empty() || k == 0 {
        return Ok(Vec::new());
    }

    let (model_name, vec_table) = active_model(conn)?;

    let embedding_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM page_embeddings WHERE model = ?1",
        [&model_name],
        |row| row.get(0),
    )?;
    if embedding_count == 0 {
        return Ok(Vec::new());
    }

    if !is_safe_identifier(&vec_table) {
        return Err(SearchError::Internal {
            message: format!("unsafe vec table name: {vec_table}"),
        });
    }

    let query_embedding = embed(query).map_err(|err| SearchError::Internal {
        message: err.to_string(),
    })?;
    let query_blob = embedding_to_blob(&query_embedding);

    let slug_expr = if canonical_slug {
        "c.name || '::' || p.slug"
    } else {
        "p.slug"
    };
    let collection_join = if canonical_slug {
        " JOIN collections c ON c.id = p.collection_id"
    } else {
        ""
    };

    let filters = VecSearchFilters {
        wing_filter,
        collection_filter,
        namespace_filter,
        include_superseded,
    };

    // Two-phase KNN retrieval (review item #10). Phase 1 over-fetches a
    // candidate set from the vec0 SIMD top-k heap via the `MATCH ... k = ?`
    // form; phase 2 joins those candidate rowids back to `page_embeddings` /
    // `pages` and applies every business filter, recomputing the cosine score
    // so it lands on the exact `1.0 - vec_distance_cosine` scale the merge
    // layer expects. If a selective filter under-fills the candidate set we
    // escalate the over-fetch (capped) and finally fall back to the original
    // full-scan query, so results are always identical to brute force.
    let mut overfetch = std::cmp::max(
        k.saturating_mul(VEC_KNN_OVERFETCH_MULTIPLIER),
        VEC_KNN_MIN_OVERFETCH,
    );
    loop {
        if overfetch as i64 >= embedding_count {
            // Candidate set already spans the whole table; a KNN pass would be
            // strictly equivalent to the full scan, so just run the full scan.
            break;
        }

        let candidate_rowids = knn_candidate_rowids(conn, &vec_table, &query_blob, overfetch)?;
        let results = run_filtered_vec_query(
            conn,
            &vec_table,
            slug_expr,
            collection_join,
            &query_blob,
            &model_name,
            &filters,
            Some(&candidate_rowids),
            k,
        )?;

        if results.len() >= k || candidate_rowids.len() < overfetch {
            // Either we filled the page (the common case) or phase 1 returned
            // fewer rows than requested, meaning the index is exhausted and a
            // larger `k` cannot surface more candidates.
            return Ok(results);
        }

        let next = overfetch.saturating_mul(VEC_KNN_ESCALATION_MULTIPLIER);
        if next > VEC_KNN_MAX_OVERFETCH || next <= overfetch {
            break;
        }
        overfetch = next;
    }

    // Full-scan fallback: no candidate restriction.
    run_filtered_vec_query(
        conn,
        &vec_table,
        slug_expr,
        collection_join,
        &query_blob,
        &model_name,
        &filters,
        None,
        k,
    )
}

/// Initial over-fetch multiplier applied to the requested `k` for the phase-1
/// KNN candidate set.
const VEC_KNN_OVERFETCH_MULTIPLIER: usize = 8;
/// Floor for the phase-1 candidate set so small `k` values still pull a useful
/// candidate pool through selective filters.
const VEC_KNN_MIN_OVERFETCH: usize = 256;
/// Factor by which the over-fetch grows on each escalation when filters
/// under-fill the requested page.
const VEC_KNN_ESCALATION_MULTIPLIER: usize = 4;
/// Cap on the escalated over-fetch before falling back to the full scan.
const VEC_KNN_MAX_OVERFETCH: usize = 65_536;

/// Business filters shared between the two-phase KNN path and the full-scan
/// fallback. Bundled so the SQL builder stays a single function.
struct VecSearchFilters<'a> {
    wing_filter: Option<&'a str>,
    collection_filter: Option<i64>,
    namespace_filter: Option<&'a str>,
    include_superseded: bool,
}

/// Phase 1: pull the `k` nearest candidate rowids from the vec0 table via its
/// SIMD top-k heap. Returns rowids only; the cosine score is recomputed in
/// phase 2 to guarantee bit-for-bit parity with the full-scan path.
fn knn_candidate_rowids(
    conn: &Connection,
    vec_table: &str,
    query_blob: &[u8],
    k: usize,
) -> Result<Vec<i64>, SearchError> {
    let sql =
        format!("SELECT rowid, distance FROM {vec_table} WHERE embedding MATCH ?1 AND k = ?2");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params![query_blob, k as i64], |row| {
        row.get::<_, i64>(0)
    })?;
    let mut rowids = Vec::new();
    for row in rows {
        rowids.push(row?);
    }
    Ok(rowids)
}

/// Phase 2 (and the full-scan fallback): join candidate rowids — or, when
/// `candidate_rowids` is `None`, the whole table — back to `page_embeddings` /
/// `pages`, apply every business filter, and rank by recomputed cosine score.
#[expect(
    clippy::too_many_arguments,
    reason = "this is the single SQL-building seam shared by both retrieval phases; the public wrappers are the right grouping boundary"
)]
fn run_filtered_vec_query(
    conn: &Connection,
    vec_table: &str,
    slug_expr: &str,
    collection_join: &str,
    query_blob: &[u8],
    model_name: &str,
    filters: &VecSearchFilters<'_>,
    candidate_rowids: Option<&[i64]>,
    k: usize,
) -> Result<Vec<SearchResult>, SearchError> {
    let mut sql = format!(
        "SELECT {slug_expr}, p.title, p.summary, \
                MAX(1.0 - vec_distance_cosine(pev.embedding, ?1)) AS score, \
                p.wing \
         FROM {vec_table} pev \
         JOIN page_embeddings pe ON pev.rowid = pe.vec_rowid \
         JOIN pages p ON p.id = pe.page_id{collection_join} \
         WHERE pe.model = ?2 \
           AND p.quarantined_at IS NULL"
    );

    let mut params: Vec<Box<dyn ToSql>> = vec![
        Box::new(query_blob.to_vec()),
        Box::new(model_name.to_owned()),
    ];

    if let Some(wing) = filters.wing_filter {
        sql.push_str(" AND p.wing = ?3");
        params.push(Box::new(wing.to_owned()));
    }

    if let Some(collection_id) = filters.collection_filter {
        sql.push_str(" AND p.collection_id = ?");
        sql.push_str(&(params.len() + 1).to_string());
        params.push(Box::new(collection_id));
    }

    if let Some(namespace) = filters.namespace_filter {
        if namespace.is_empty() {
            sql.push_str(" AND p.namespace = ?");
            sql.push_str(&(params.len() + 1).to_string());
            params.push(Box::new(String::new()));
        } else {
            sql.push_str(" AND (p.namespace = ?");
            sql.push_str(&(params.len() + 1).to_string());
            sql.push_str(" OR p.namespace = '')");
            params.push(Box::new(namespace.to_owned()));
        }
    }

    if !filters.include_superseded {
        sql.push_str(" AND p.superseded_by IS NULL");
    }

    if let Some(rowids) = candidate_rowids {
        if rowids.is_empty() {
            return Ok(Vec::new());
        }
        // Rowids are trusted i64s read from `page_embeddings.vec_rowid`, so we
        // inline them as integer literals rather than binding one parameter
        // each: the over-fetched candidate set can far exceed SQLite's bound
        // variable limit (`SQLITE_MAX_VARIABLE_NUMBER`).
        use std::fmt::Write as _;
        sql.push_str(" AND pev.rowid IN (");
        for (index, rowid) in rowids.iter().enumerate() {
            if index > 0 {
                sql.push(',');
            }
            // i64 Display only ever emits digits and a leading '-', so this is
            // injection-safe by construction.
            let _ = write!(sql, "{rowid}");
        }
        sql.push(')');
    }

    let limit_index = params.len() + 1;
    sql.push_str(" GROUP BY p.id ORDER BY score DESC LIMIT ?");
    sql.push_str(&limit_index.to_string());
    params.push(Box::new(k as i64));

    let param_refs: Vec<&dyn ToSql> = params.iter().map(|param| param.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok(SearchResult {
            slug: row.get(0)?,
            title: row.get(1)?,
            summary: row.get(2)?,
            score: row.get(3)?,
            wing: row.get(4)?,
        })
    })?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
}

fn active_model(conn: &Connection) -> Result<(String, String), SearchError> {
    conn.query_row(
        "SELECT name, vec_table FROM embedding_models WHERE active = 1 LIMIT 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => SearchError::Internal {
            message: "no active embedding model configured".to_owned(),
        },
        other => SearchError::from(other),
    })
}

/// Recomputes and replaces all chunk embeddings for `page_id` under the
/// currently active model, returning the number of chunks indexed. Drops any
/// prior embeddings transactionally.
pub fn refresh_page_embeddings(
    conn: &Connection,
    page_id: i64,
    page: &crate::core::types::Page,
) -> Result<usize, SearchError> {
    let (model_name, vec_table) = active_model(conn)?;
    if !is_safe_identifier(&vec_table) {
        return Err(SearchError::Internal {
            message: format!("unsafe vec table name: {vec_table}"),
        });
    }

    let chunks = chunk_page(page);
    replace_page_embeddings(conn, page_id, &model_name, &vec_table, &chunks)?;
    Ok(chunks.len())
}

/// Encodes a float embedding into the little-endian byte blob format expected
/// by the `sqlite-vec` virtual tables.
pub fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

fn is_safe_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn replace_page_embeddings(
    conn: &Connection,
    page_id: i64,
    model_name: &str,
    vec_table: &str,
    chunks: &[crate::core::types::Chunk],
) -> Result<(), SearchError> {
    let tx = conn.unchecked_transaction()?;

    let existing_rowids = existing_vec_rowids(&tx, page_id, model_name)?;
    let delete_vec_sql = format!("DELETE FROM {vec_table} WHERE rowid = ?1");
    for vec_rowid in existing_rowids {
        tx.execute(&delete_vec_sql, [vec_rowid])?;
    }

    tx.execute(
        "DELETE FROM page_embeddings WHERE page_id = ?1 AND model = ?2",
        rusqlite::params![page_id, model_name],
    )?;

    let insert_vec_sql = format!("INSERT INTO {vec_table}(embedding) VALUES (?1)");
    for (chunk_index, chunk) in chunks.iter().enumerate() {
        let embedding = embed(&chunk.content).map_err(|err| SearchError::Internal {
            message: err.to_string(),
        })?;
        let embedding_blob = embedding_to_blob(&embedding);
        tx.execute(&insert_vec_sql, rusqlite::params![embedding_blob])?;
        let vec_rowid = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO page_embeddings \
                 (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, \
                  content_hash, token_count, heading_path) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                page_id,
                model_name,
                vec_rowid,
                chunk.chunk_type,
                chunk_index as i64,
                chunk.content,
                chunk.content_hash,
                chunk.token_count as i64,
                chunk.heading_path,
            ],
        )?;
    }

    tx.commit()?;
    Ok(())
}

fn existing_vec_rowids(
    conn: &Connection,
    page_id: i64,
    model_name: &str,
) -> Result<Vec<i64>, SearchError> {
    let mut stmt = conn.prepare(
        "SELECT vec_rowid FROM page_embeddings WHERE page_id = ?1 AND model = ?2 ORDER BY chunk_index",
    )?;
    let rows = stmt.query_map(rusqlite::params![page_id, model_name], |row| row.get(0))?;

    let mut rowids = Vec::new();
    for row in rows {
        rowids.push(row?);
    }
    Ok(rowids)
}

/// Deletes every `page_embeddings_vec_*` row backing the given pages, across
/// all registered embedding models, **before** the pages themselves are
/// deleted. vec0 virtual tables do not participate in SQLite foreign-key
/// cascades, so bulk page deletes (namespace destroy, collection purge,
/// reconciler hard-delete, quarantine discard) would otherwise orphan their
/// vectors permanently (review item #10). Call this inside the same
/// transaction as the page delete and while the `page_embeddings` rows still
/// exist, since the vec rowids are resolved through them. Returns the number
/// of vec rows deleted.
pub fn delete_page_vec_rows(conn: &Connection, page_ids: &[i64]) -> Result<usize, SearchError> {
    if page_ids.is_empty() {
        return Ok(0);
    }

    // Resolve every (vec_table, vec_rowid) pair the pages reference, grouped by
    // the model's vec table so we issue one DELETE per table.
    let mut stmt = conn.prepare(
        "SELECT em.vec_table, pe.vec_rowid \
         FROM page_embeddings pe \
         JOIN embedding_models em ON em.name = pe.model \
         WHERE pe.page_id = ?1",
    )?;

    let mut by_table: std::collections::BTreeMap<String, Vec<i64>> =
        std::collections::BTreeMap::new();
    for page_id in page_ids {
        let rows = stmt.query_map([page_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (vec_table, vec_rowid) = row?;
            by_table.entry(vec_table).or_default().push(vec_rowid);
        }
    }
    drop(stmt);

    let mut deleted = 0usize;
    for (vec_table, rowids) in by_table {
        if !is_safe_identifier(&vec_table) {
            return Err(SearchError::Internal {
                message: format!("unsafe vec table name: {vec_table}"),
            });
        }
        let delete_sql = format!("DELETE FROM {vec_table} WHERE rowid = ?1");
        for rowid in rowids {
            deleted += conn.execute(&delete_sql, [rowid])?;
        }
    }

    Ok(deleted)
}

/// Sweeps `page_embeddings_vec_*` rows whose backing `page_embeddings` join row
/// no longer exists, across every registered embedding model's vec table. This
/// is the janitor counterpart to [`delete_page_vec_rows`]: it reclaims vectors
/// that predate the vec-aware delete paths (or were orphaned by a crash between
/// the `page_embeddings` cascade and a vec delete). Returns the number of
/// orphaned vec rows removed.
pub fn sweep_orphaned_vec_rows(conn: &Connection) -> Result<usize, SearchError> {
    let vec_tables: Vec<String> = {
        let mut stmt = conn.prepare("SELECT vec_table FROM embedding_models")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut tables = Vec::new();
        for row in rows {
            tables.push(row?);
        }
        tables
    };

    let mut deleted = 0usize;
    for vec_table in vec_tables {
        if !is_safe_identifier(&vec_table) {
            return Err(SearchError::Internal {
                message: format!("unsafe vec table name: {vec_table}"),
            });
        }
        // vec0 virtual tables do not support correlated DELETE subqueries
        // against arbitrary tables on every build, so collect the orphan
        // rowids first, then delete them by rowid.
        let select_sql = format!(
            "SELECT v.rowid FROM {vec_table} v \
             WHERE NOT EXISTS (SELECT 1 FROM page_embeddings pe WHERE pe.vec_rowid = v.rowid)"
        );
        let orphan_rowids: Vec<i64> = {
            let mut stmt = conn.prepare(&select_sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            ids
        };
        let delete_sql = format!("DELETE FROM {vec_table} WHERE rowid = ?1");
        for rowid in orphan_rowids {
            deleted += conn.execute(&delete_sql, [rowid])?;
        }
    }

    Ok(deleted)
}

fn normalize(values: &mut [f32]) -> Result<(), InferenceError> {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return Err(InferenceError::Internal {
            message: "embedding norm is zero".to_owned(),
        });
    }

    for value in values {
        *value /= norm;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::core::db;
    use crate::core::types::Page;
    #[cfg(feature = "online-model")]
    use std::io::{Read, Write};
    #[cfg(feature = "online-model")]
    use std::net::TcpListener;
    #[cfg(feature = "online-model")]
    use std::thread;

    // Guard for tests that mutate process-global env vars (QUAID_HF_BASE_URL,
    // QUAID_MODEL_CACHE_DIR). Rust tests run in parallel by default; without
    // this mutex those tests can observe each other's env-var changes and flake.
    static ENV_MUTATION_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    fn env_mutation_lock() -> &'static std::sync::Mutex<()> {
        ENV_MUTATION_LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

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

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_memory.db");
        let conn = db::open(db_path.to_str().expect("utf8 path")).expect("open db");
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn resolve_model_supports_standard_aliases() {
        let cases = [
            ("small", "BAAI/bge-small-en-v1.5", 384, "small"),
            ("base", "BAAI/bge-base-en-v1.5", 768, "base"),
            ("large", "BAAI/bge-large-en-v1.5", 1024, "large"),
            ("m3", "BAAI/bge-m3", 1024, "m3"),
        ];

        for (input, expected_id, expected_dim, expected_alias) in cases {
            let model = resolve_model(input);
            assert_eq!(model.model_id, expected_id);
            assert_eq!(model.embedding_dim, expected_dim);
            assert_eq!(model.alias, expected_alias);
            assert!(model.sha256_hashes.is_some());
        }
    }

    #[test]
    fn resolve_model_preserves_custom_huggingface_ids() {
        let standard = resolve_model("BAAI/bge-large-en-v1.5");
        assert_eq!(standard.alias, "large");
        assert_eq!(standard.model_id, "BAAI/bge-large-en-v1.5");
        assert_eq!(standard.embedding_dim, 1024);

        let model = resolve_model("org/custom-embedder");
        assert_eq!(model.alias, "custom");
        assert_eq!(model.model_id, "org/custom-embedder");
        assert_eq!(model.embedding_dim, 0);
        assert!(model.sha256_hashes.is_none());
    }

    #[test]
    fn known_model_helpers_cover_aliases_and_repo_ids() {
        let aliases = known_embedding_models()
            .into_iter()
            .map(|model| model.alias)
            .collect::<Vec<_>>();
        assert_eq!(aliases, vec!["small", "base", "large", "m3"]);
        assert_eq!(
            resolve_known_embedding_model("BAAI/bge-base-en-v1.5")
                .expect("known repo")
                .alias,
            "base"
        );
        assert!(resolve_known_embedding_model("org/custom").is_none());
        assert_eq!(resolve_model("BAAI/bge-small-en-v1.5").alias, "small");
        assert_eq!(resolve_model("BAAI/bge-base-en-v1.5").alias, "base");
        assert_eq!(resolve_model("BAAI/bge-m3").alias, "m3");
    }

    #[test]
    fn model_config_helper_methods_reflect_aliases_and_dimensions() {
        let small = default_model();
        assert_eq!(small.vec_table(), "page_embeddings_vec_384");
        assert_eq!(small.embedding_model_name(), "BAAI/bge-small-en-v1.5");
        assert_eq!(small.model_hint(), "small");
        assert!(small.is_small());
        assert!(!small.needs_dimension_hydration());

        let custom = resolve_model("org/custom-model");
        assert_eq!(custom.model_hint(), "org/custom-model");
        assert!(!custom.is_small());
        assert!(custom.needs_dimension_hydration());
    }

    #[test]
    fn resolve_requested_model_uses_default_model_when_none_is_provided() {
        let model = resolve_requested_model(None);

        assert_eq!(model, default_model());
    }

    #[cfg(feature = "embedded-model")]
    #[test]
    fn coerce_model_for_build_falls_back_to_small_on_embedded_builds() {
        let coerced = coerce_model_for_build(&resolve_model("large"));

        assert_eq!(coerced, default_model());
    }

    #[cfg(not(feature = "online-model"))]
    #[test]
    fn hydrate_model_config_rejects_custom_models_without_online_support() {
        let err = hydrate_model_config(&resolve_model("org/custom-model")).unwrap_err();

        assert!(err.contains("requires the online-model build"));
    }

    #[test]
    fn embedding_model_debug_includes_hash_shim_metadata() {
        let model = EmbeddingModel {
            config: resolve_model("org/custom-model"),
            backend: EmbeddingBackend::HashShim,
        };

        let debug = format!("{model:?}");

        assert!(debug.contains("HashShim"));
        assert!(debug.contains("org/custom-model"));
    }

    #[test]
    fn embedding_evidence_kind_reports_loaded_backend() {
        configure_runtime_model(default_model());
        let kind = embedding_evidence_kind().expect("evidence kind");

        assert!(matches!(
            kind,
            EmbeddingEvidenceKind::Semantic | EmbeddingEvidenceKind::HashShim
        ));
    }

    #[test]
    fn embed_returns_normalized_vector_of_expected_length() {
        // Force hash shim so this test never triggers a real HuggingFace
        // download in CI (download attempt would block for up to 300s).
        let _env_guard = env_mutation_lock()
            .lock()
            .expect("env mutation lock poisoned");
        let _force_hash_shim = EnvVarGuard::set("QUAID_FORCE_HASH_SHIM", "1");
        configure_runtime_model(default_model());
        // Reset loaded model so the env var is respected.
        model_runtime().lock().expect("lock").loaded = None;
        let embedding = embed("Alice works at Acme Corp").expect("embed text");
        let norm = embedding
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();

        assert_eq!(embedding.len(), DEFAULT_EMBEDDING_DIMENSIONS);
        assert!((norm - 1.0).abs() < 1e-5, "unexpected norm: {norm}");
    }

    #[test]
    fn embed_hash_shim_uses_runtime_dimension() {
        let embedding = embed_hash_shim("test input", 1024).expect("hash shim");
        assert_eq!(embedding.len(), 1024);
    }

    #[test]
    fn embed_recovers_from_poisoned_model_runtime_mutex() {
        // Locks env_mutation_lock so we can safely toggle the hash-shim env
        // var without racing other tests, mirroring the pattern at
        // embed_returns_normalized_vector_of_expected_length.
        let _env_guard = env_mutation_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _force_hash_shim = EnvVarGuard::set("QUAID_FORCE_HASH_SHIM", "1");

        // Poison MODEL_RUNTIME by panicking inside its guard on a worker
        // thread; .join() captures the panic so it does not propagate.
        let join = std::thread::spawn(|| {
            let _g = model_runtime().lock().unwrap();
            panic!("intentional");
        })
        .join();
        assert!(join.is_err(), "worker thread did not panic");

        // After poisoning, both APIs that previously panicked on poisoned
        // mutex acquisition must now recover and return Ok(_).
        configure_runtime_model(default_model());
        // Reset loaded so the env var is honored on the next embed call.
        model_runtime()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded = None;
        let result = embed("recovery probe");
        assert!(result.is_ok(), "embed failed after poison: {result:?}");

        // Reset state so subsequent tests in the same process are not
        // observably contaminated. clear_poison restores the mutex to a
        // healthy state for sibling tests (e.g.
        // embed_returns_normalized_vector_of_expected_length) that still
        // call `.expect("lock")` on this same static mutex.
        model_runtime()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .loaded = None;
        model_runtime().clear_poison();
    }

    #[test]
    fn embed_returns_error_for_empty_input() {
        let err = embed("   ").expect_err("empty input should fail");
        assert!(matches!(err, InferenceError::EmptyInput));
    }

    #[test]
    fn search_vec_on_empty_db_returns_empty_vec() {
        let conn = open_test_db();
        let results = search_vec("board member tech company", 5, None, None, &conn)
            .expect("empty db search should succeed");

        assert!(results.is_empty());
    }

    #[test]
    fn search_vec_short_circuits_blank_query_and_zero_limit() {
        let conn = open_test_db();

        assert!(search_vec("   ", 5, None, None, &conn).unwrap().is_empty());
        assert!(search_vec("query", 0, None, None, &conn)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn active_model_returns_error_when_no_active_row_exists() {
        let conn = open_test_db();
        conn.execute("UPDATE embedding_models SET active = 0", [])
            .expect("clear active model");

        let err = active_model(&conn).unwrap_err();

        assert!(matches!(err, SearchError::Internal { .. }));
    }

    #[test]
    fn search_vec_rejects_unsafe_vec_table_names() {
        let conn = open_test_db();
        let model_name: String = conn
            .query_row(
                "SELECT name FROM embedding_models WHERE active = 1",
                [],
                |row| row.get(0),
            )
            .expect("fetch active model");
        conn.execute(
            "UPDATE embedding_models SET vec_table = 'page-embeddings-vec-384' WHERE active = 1",
            [],
        )
        .expect("set unsafe vec table");
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES ('people/alice', 'person', 'Alice', 'Founder', '', '', '{}', 'people', '', 1)",
            [],
        )
        .expect("insert page");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, ?2, 1, 'truth_section', 0, 'startup founder', 'hash', 2, 'State')",
            rusqlite::params![page_id, model_name],
        )
        .expect("insert embedding metadata");

        let err = search_vec("startup founder", 5, None, None, &conn).unwrap_err();

        assert!(matches!(err, SearchError::Internal { .. }));
    }

    #[test]
    fn search_vec_returns_ranked_results_from_vec_table() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, 'person', ?2, ?3, '', '', '{}', ?4, '', 1)",
            rusqlite::params!["people/alice", "Alice", "Founder", "people"],
        )
        .expect("insert page");

        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");

        let query_embedding = embed("startup founder").expect("embed query");
        let query_blob = embedding_to_blob(&query_embedding);
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![1_i64, query_blob],
        )
        .expect("insert vec row");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 1, 'truth_section', 0, 'startup founder', 'hash', 2, 'State')",
            rusqlite::params![page_id],
        )
        .expect("insert embedding metadata");

        let results = search_vec("startup founder", 5, Some("people"), None, &conn)
            .expect("vector search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
        assert!(
            results[0].score > 0.99,
            "unexpected score: {}",
            results[0].score
        );
    }

    #[test]
    fn canonical_vector_search_applies_collection_and_namespace_filters() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, namespace, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version, collection_id) \
             VALUES (?1, ?2, 'person', ?3, ?4, '', '', '{}', ?5, '', 1, 1)",
            rusqlite::params!["people/alice", "alpha", "Alice", "Founder", "people"],
        )
        .expect("insert namespaced page");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");
        let query_blob = embedding_to_blob(&embed("startup founder").expect("embed query"));
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![1_i64, query_blob],
        )
        .expect("insert vec row");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 1, 'truth_section', 0, 'startup founder', 'hash', 2, 'State')",
            rusqlite::params![page_id],
        )
        .expect("insert embedding metadata");

        let results = search_vec_canonical_with_namespace_filtered(
            "startup founder",
            5,
            Some("people"),
            Some(1),
            Some("alpha"),
            true,
            &conn,
        )
        .expect("canonical vector search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "default::people/alice");
    }

    #[test]
    fn search_vec_excludes_quarantined_pages_from_results() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, 'person', ?2, ?3, '', '', '{}', 'people', '', 1)",
            rusqlite::params!["people/alice", "Alice", "Founder"],
        )
        .expect("insert active page");
        conn.execute(
            "INSERT INTO pages (slug, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version, quarantined_at) \
             VALUES (?1, 'person', ?2, ?3, '', '', '{}', 'people', '', 1, '2026-04-22T00:00:00Z')",
            rusqlite::params!["people/bob", "Bob", "Founder"],
        )
        .expect("insert quarantined page");

        let active_page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/alice'",
                [],
                |row| row.get(0),
            )
            .expect("fetch active page id");
        let quarantined_page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'people/bob'",
                [],
                |row| row.get(0),
            )
            .expect("fetch quarantined page id");

        let query_embedding = embed("startup founder").expect("embed query");
        let query_blob = embedding_to_blob(&query_embedding);
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![1_i64, &query_blob],
        )
        .expect("insert active vec row");
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![2_i64, &query_blob],
        )
        .expect("insert quarantined vec row");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 1, 'truth_section', 0, 'startup founder', 'hash-a', 2, 'State')",
            rusqlite::params![active_page_id],
        )
        .expect("insert active embedding metadata");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 2, 'truth_section', 0, 'startup founder', 'hash-b', 2, 'State')",
            rusqlite::params![quarantined_page_id],
        )
        .expect("insert quarantined embedding metadata");

        let results = search_vec("startup founder", 5, Some("people"), None, &conn)
            .expect("vector search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
    }

    #[test]
    fn refresh_page_embeddings_replaces_existing_rows_and_returns_chunk_count() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'note', 'Refresh', '', 'old truth', '', '{}', 'notes', '', 1)",
            rusqlite::params!["notes/refresh", uuid::Uuid::now_v7().to_string()],
        )
        .expect("insert page");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/refresh'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");
        let stale_embedding = embedding_to_blob(&embed("stale embedding").expect("embed stale"));
        conn.execute(
            "INSERT INTO page_embeddings_vec_384(rowid, embedding) VALUES (?1, ?2)",
            rusqlite::params![1_i64, stale_embedding],
        )
        .expect("insert stale vec row");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 1, 'truth_section', 0, 'stale chunk', 'stale-hash', 2, 'State')",
            rusqlite::params![page_id],
        )
        .expect("insert stale metadata");

        let page = Page {
            slug: "notes/refresh".to_owned(),
            uuid: uuid::Uuid::now_v7().to_string(),
            page_type: "note".to_owned(),
            superseded_by: None,
            title: "Refresh".to_owned(),
            summary: String::new(),
            compiled_truth: "## State\nFresh truth\n".to_owned(),
            timeline: "- 2026-04-28: refreshed timeline entry".to_owned(),
            frontmatter: crate::core::types::Frontmatter::new(),
            wing: "notes".to_owned(),
            room: String::new(),
            version: 2,
            created_at: "2026-04-28T00:00:00Z".to_owned(),
            updated_at: "2026-04-28T00:00:00Z".to_owned(),
            truth_updated_at: "2026-04-28T00:00:00Z".to_owned(),
            timeline_updated_at: "2026-04-28T00:00:00Z".to_owned(),
        };
        let expected_chunks = chunk_page(&page).len();

        let refreshed = refresh_page_embeddings(&conn, page_id, &page).expect("refresh embeddings");

        assert_eq!(refreshed, expected_chunks);
        let stale_vec_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings_vec_384 WHERE rowid = 1",
                [],
                |row| row.get(0),
            )
            .expect("count stale vec rows");
        assert_eq!(stale_vec_count, 0);
        let metadata_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM page_embeddings WHERE page_id = ?1 AND model = 'BAAI/bge-small-en-v1.5'",
                rusqlite::params![page_id],
                |row| row.get(0),
        )
        .expect("count refreshed metadata rows");
        assert_eq!(metadata_count as usize, expected_chunks);
    }

    #[test]
    fn refresh_page_embeddings_rejects_unsafe_vec_table_names() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'note', 'Refresh', '', 'truth', '', '{}', 'notes', '', 1)",
            rusqlite::params!["notes/unsafe-refresh", uuid::Uuid::now_v7().to_string()],
        )
        .expect("insert page");
        conn.execute(
            "UPDATE embedding_models SET vec_table = 'page-embeddings-vec-384' WHERE active = 1",
            [],
        )
        .expect("set unsafe vec table");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/unsafe-refresh'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");
        let page = Page {
            slug: "notes/unsafe-refresh".to_owned(),
            uuid: uuid::Uuid::now_v7().to_string(),
            page_type: "note".to_owned(),
            superseded_by: None,
            title: "Refresh".to_owned(),
            summary: String::new(),
            compiled_truth: "truth".to_owned(),
            timeline: String::new(),
            frontmatter: crate::core::types::Frontmatter::new(),
            wing: "notes".to_owned(),
            room: String::new(),
            version: 1,
            created_at: "2026-04-28T00:00:00Z".to_owned(),
            updated_at: "2026-04-28T00:00:00Z".to_owned(),
            truth_updated_at: "2026-04-28T00:00:00Z".to_owned(),
            timeline_updated_at: "2026-04-28T00:00:00Z".to_owned(),
        };

        let err = refresh_page_embeddings(&conn, page_id, &page).unwrap_err();

        assert!(matches!(err, SearchError::Internal { .. }));
        assert!(err.to_string().contains("unsafe vec table name"));
    }

    #[test]
    fn existing_vec_rowids_returns_sorted_rowids_for_page_and_model() {
        let conn = open_test_db();
        conn.execute(
            "INSERT INTO pages (slug, uuid, type, title, summary, compiled_truth, timeline, frontmatter, wing, room, version) \
             VALUES (?1, ?2, 'note', 'Refresh', '', 'truth', '', '{}', 'notes', '', 1)",
            rusqlite::params!["notes/rowids", uuid::Uuid::now_v7().to_string()],
        )
        .expect("insert page");
        let page_id: i64 = conn
            .query_row(
                "SELECT id FROM pages WHERE slug = 'notes/rowids'",
                [],
                |row| row.get(0),
            )
            .expect("fetch page id");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 22, 'truth_section', 1, 'second', 'hash-b', 1, 'B')",
            rusqlite::params![page_id],
        )
        .expect("insert second rowid");
        conn.execute(
            "INSERT INTO page_embeddings (page_id, model, vec_rowid, chunk_type, chunk_index, chunk_text, content_hash, token_count, heading_path) \
             VALUES (?1, 'BAAI/bge-small-en-v1.5', 11, 'truth_section', 0, 'first', 'hash-a', 1, 'A')",
            rusqlite::params![page_id],
        )
        .expect("insert first rowid");

        let rowids = existing_vec_rowids(&conn, page_id, "BAAI/bge-small-en-v1.5").unwrap();

        assert_eq!(rowids, vec![11, 22]);
    }

    #[test]
    fn embedding_to_blob_writes_little_endian_f32_values() {
        let blob = embedding_to_blob(&[1.0, -2.5]);

        assert_eq!(blob.len(), 8);
        assert_eq!(&blob[..4], &1.0_f32.to_le_bytes());
        assert_eq!(&blob[4..], &(-2.5_f32).to_le_bytes());
    }

    #[test]
    fn normalize_rejects_zero_vectors() {
        let mut values = [0.0_f32, 0.0_f32];
        let err = normalize(&mut values).unwrap_err();

        assert!(matches!(err, InferenceError::Internal { .. }));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn hydrate_model_config_can_use_mock_huggingface_downloads() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind listener");
        let address = listener.local_addr().expect("listener addr");
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().expect("accept connection");
                let mut buffer = [0_u8; 2048];
                let size = stream.read(&mut buffer).expect("read request");
                let request = String::from_utf8_lossy(&buffer[..size]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("request path");

                let body = if path.ends_with("/config.json") {
                    "{\n  \"hidden_size\": 1536,\n  \"max_position_embeddings\": 2048,\n  \"model_type\": \"bert\"\n}\n".to_owned()
                } else {
                    "{}".to_owned()
                };

                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                stream
                    .write_all(response.as_bytes())
                    .expect("write response");
            }
        });

        let cache_dir = tempfile::TempDir::new().expect("create cache dir");

        // Hold the env-mutation lock for the duration of the test so parallel
        // tests cannot observe our QUAID_HF_BASE_URL / QUAID_MODEL_CACHE_DIR
        // changes. Restore previous values (if any) on drop.
        let _env_guard = env_mutation_lock()
            .lock()
            .expect("env mutation lock poisoned");
        let _base_url = EnvVarGuard::set("QUAID_HF_BASE_URL", format!("http://{}", address));
        let _cache_dir = EnvVarGuard::set("QUAID_MODEL_CACHE_DIR", cache_dir.path());

        let model = resolve_model("org/custom-model");
        let hydrated = hydrate_model_config(&model).expect("hydrate custom model");
        server.join().expect("join mock server");

        assert_eq!(hydrated.model_id, "org/custom-model");
        assert_eq!(hydrated.embedding_dim, 1536);
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn validate_model_id_rejects_extra_slashes_and_query_chars() {
        let err = validate_model_id("org/extra/model").unwrap_err();
        assert!(err.contains("exactly one '/' separator"));

        let err = validate_model_id("org/model?rev=main").unwrap_err();
        assert!(err.contains("not allowed"));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn cache_dir_name_falls_back_to_hash_for_degenerate_inputs() {
        let fallback = cache_dir_name_from_model_id("../..");
        assert!(fallback.starts_with("custom-"));

        let fallback = cache_dir_name_from_model_id("org//model");
        assert!(fallback.starts_with("custom-"));
    }

    #[cfg(feature = "online-model")]
    #[test]
    fn read_max_position_embeddings_from_config_uses_config_json_value() {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let config_path = dir.path().join("config.json");
        std::fs::write(
            &config_path,
            "{\n  \"hidden_size\": 1024,\n  \"max_position_embeddings\": 4096,\n  \"model_type\": \"xlm-roberta\"\n}\n",
        )
        .expect("write config");

        let max_len = read_max_position_embeddings_from_config(&config_path).expect("read max len");

        assert_eq!(max_len, 4096);
    }
}
