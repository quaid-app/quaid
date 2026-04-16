//! Inference — text embedding and vector search via BGE-small-en-v1.5.
//!
//! Uses Candle (pure Rust ML) to run the BAAI/bge-small-en-v1.5 BERT model on
//! CPU. Two compile-time channels are supported:
//!
//! - `embedded-model` — airgapped build with embedded model assets
//! - `online-model` — online build with first-use download + cache
//!
//! If the model cannot be loaded (no network, no cache, missing feature flag),
//! the system falls back to a SHA-256 hash-based shim that satisfies the API
//! contract (384-dim, L2-normalised) but produces non-semantic vectors.
//!
//! The public API (`embed`, `search_vec`, `ensure_model`, `embedding_to_blob`)
//! is stable regardless of which backend is active.

use std::sync::OnceLock;

#[cfg(feature = "online-model")]
use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config as BertConfig};
use rusqlite::types::ToSql;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tokenizers::Tokenizer;

use super::types::{InferenceError, SearchError, SearchResult};

#[cfg(all(feature = "embedded-model", feature = "online-model"))]
compile_error!("Enable only one model channel: `embedded-model` or `online-model`.");

const EMBEDDING_DIMENSIONS: usize = 384;
const HASH_CHUNK_COUNT: usize = EMBEDDING_DIMENSIONS / 32;
#[cfg(feature = "online-model")]
const MODEL_ID: &str = "BAAI/bge-small-en-v1.5";

static MODEL: OnceLock<EmbeddingModel> = OnceLock::new();

/// BGE-small-en-v1.5 embedding model backed by Candle, with SHA-256 fallback.
pub struct EmbeddingModel {
    backend: EmbeddingBackend,
}

enum EmbeddingBackend {
    /// Real BGE-small-en-v1.5 BERT model via Candle.
    Candle {
        model: Box<BertModel>,
        tokenizer: Box<Tokenizer>,
        device: Device,
    },
    /// SHA-256 hash-based fallback (non-semantic, deterministic).
    HashShim,
}

impl std::fmt::Debug for EmbeddingModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.backend {
            EmbeddingBackend::Candle { .. } => f
                .debug_struct("EmbeddingModel")
                .field("backend", &"Candle(BGE-small-en-v1.5)")
                .finish(),
            EmbeddingBackend::HashShim => f
                .debug_struct("EmbeddingModel")
                .field("backend", &"HashShim")
                .finish(),
        }
    }
}

impl EmbeddingModel {
    fn new() -> Self {
        match Self::try_load_candle() {
            Ok(backend) => Self { backend },
            Err(err) => {
                eprintln!(
                    "Warning: BGE-small model not available ({err}), \
                     using hash-based embeddings. Build the default airgapped channel with \
                     `cargo build --release` or use the online channel with \
                     `cargo build --release --no-default-features --features bundled,online-model`, \
                     then run `gbrain embed --all`."
                );
                Self {
                    backend: EmbeddingBackend::HashShim,
                }
            }
        }
    }

    fn try_load_candle() -> Result<EmbeddingBackend, String> {
        #[cfg(feature = "embedded-model")]
        {
            return load_embedded_backend();
        }

        #[cfg(feature = "online-model")]
        {
            return load_online_backend();
        }

        #[cfg(not(any(feature = "embedded-model", feature = "online-model")))]
        {
            Err("no model channel enabled".to_owned())
        }
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, InferenceError> {
        match &self.backend {
            EmbeddingBackend::Candle {
                model,
                tokenizer,
                device,
            } => embed_candle(text, model, tokenizer, device),
            EmbeddingBackend::HashShim => embed_hash_shim(text),
        }
    }
}

#[cfg(feature = "embedded-model")]
fn load_embedded_backend() -> Result<EmbeddingBackend, String> {
    let config: BertConfig =
        serde_json::from_slice(include_bytes!(env!("GBRAIN_EMBEDDED_CONFIG_PATH")))
            .map_err(|e| format!("parse embedded config.json: {e}"))?;
    let tokenizer = Tokenizer::from_bytes(include_bytes!(env!("GBRAIN_EMBEDDED_TOKENIZER_PATH")))
        .map_err(|e| format!("load embedded tokenizer: {e}"))?;
    let device = Device::Cpu;
    let vb = VarBuilder::from_slice_safetensors(
        include_bytes!(env!("GBRAIN_EMBEDDED_MODEL_PATH")),
        DType::F32,
        &device,
    )
    .map_err(|e| format!("load embedded model weights: {e}"))?;
    let model = BertModel::load(vb, &config).map_err(|e| format!("build BERT model: {e}"))?;

    Ok(EmbeddingBackend::Candle {
        model: Box::new(model),
        tokenizer: Box::new(tokenizer),
        device,
    })
}

#[cfg(feature = "online-model")]
fn load_online_backend() -> Result<EmbeddingBackend, String> {
    let (config_path, tokenizer_path, model_path) =
        download_model_files().map_err(|e| format!("model download: {e}"))?;

    let config_text =
        std::fs::read_to_string(&config_path).map_err(|e| format!("read config.json: {e}"))?;
    let config: BertConfig =
        serde_json::from_str(&config_text).map_err(|e| format!("parse config.json: {e}"))?;
    let tokenizer =
        Tokenizer::from_file(&tokenizer_path).map_err(|e| format!("load tokenizer: {e}"))?;
    let device = Device::Cpu;
    let vb = unsafe {
        VarBuilder::from_mmaped_safetensors(&[model_path], DType::F32, &device)
            .map_err(|e| format!("load model weights: {e}"))?
    };
    let model = BertModel::load(vb, &config).map_err(|e| format!("build BERT model: {e}"))?;

    Ok(EmbeddingBackend::Candle {
        model: Box::new(model),
        tokenizer: Box::new(tokenizer),
        device,
    })
}

/// Run the BERT forward pass and mean-pool + L2-normalize the output.
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

    let ids = encoding.get_ids();
    let mask = encoding.get_attention_mask();

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

    // Mean pooling over token dimension, masked by attention_mask
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

    // L2 normalize
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

    let embedding = normalized
        .squeeze(0)
        .and_then(|t| t.to_vec1::<f32>())
        .map_err(|e| InferenceError::Internal {
            message: format!("to_vec: {e}"),
        })?;

    Ok(embedding)
}

/// Download BGE-small-en-v1.5 model files into the local GigaBrain cache.
#[cfg(feature = "online-model")]
fn download_model_files(
) -> Result<(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf), String> {
    let cache_dir = model_cache_dir()?;

    if let Some(paths) = existing_model_paths(&cache_dir) {
        return Ok(paths);
    }

    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| format!("create model cache {}: {e}", cache_dir.display()))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .user_agent("gigabrain-runtime/0.9.1")
        .build()
        .map_err(|e| format!("build download client: {e}"))?;

    for file_name in ["config.json", "tokenizer.json", "model.safetensors"] {
        download_model_file(&client, file_name, &cache_dir)?;
    }

    existing_model_paths(&cache_dir).ok_or_else(|| {
        format!(
            "BGE-small model files missing after download in {}",
            cache_dir.display()
        )
    })
}

#[cfg(feature = "online-model")]
fn download_model_file(
    client: &reqwest::blocking::Client,
    file_name: &str,
    cache_dir: &Path,
) -> Result<(), String> {
    let destination = cache_dir.join(file_name);
    let temp_destination = cache_dir.join(format!("{file_name}.download"));
    let url = format!("https://huggingface.co/{MODEL_ID}/resolve/main/{file_name}");

    let mut response = client
        .get(&url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|e| format!("download {url}: {e}"))?;

    let mut file = std::fs::File::create(&temp_destination)
        .map_err(|e| format!("create {}: {e}", temp_destination.display()))?;
    std::io::copy(&mut response, &mut file)
        .map_err(|e| format!("write {}: {e}", temp_destination.display()))?;
    std::fs::rename(&temp_destination, &destination)
        .map_err(|e| format!("rename {}: {e}", destination.display()))?;

    Ok(())
}

#[cfg(feature = "online-model")]
fn model_cache_dir() -> Result<PathBuf, String> {
    dirs::home_dir()
        .map(|home| {
            home.join(".gbrain")
                .join("models")
                .join("bge-small-en-v1.5")
        })
        .ok_or_else(|| "could not resolve home directory for model cache".to_owned())
}

#[cfg(feature = "online-model")]
fn existing_model_paths(cache_dir: &Path) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let config = cache_dir.join("config.json");
    let tokenizer = cache_dir.join("tokenizer.json");
    let model = cache_dir.join("model.safetensors");

    (config.is_file() && tokenizer.is_file() && model.is_file())
        .then_some((config, tokenizer, model))
}

/// SHA-256 hash-based fallback for when the real model is unavailable.
fn embed_hash_shim(text: &str) -> Result<Vec<f32>, InferenceError> {
    let mut embedding = vec![0.0; EMBEDDING_DIMENSIONS];

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
    for chunk_index in 0..HASH_CHUNK_COUNT {
        let mut hasher = Sha256::new();
        hasher.update(token.as_bytes());
        hasher.update((token_index as u64).to_le_bytes());
        hasher.update((chunk_index as u64).to_le_bytes());
        let digest = hasher.finalize();
        let start = chunk_index * 32;

        for (offset, byte) in digest.iter().enumerate() {
            let centered = (*byte as f32 / 127.5) - 1.0;
            embedding[start + offset] += centered;
        }
    }
}

/// Lazily initialises the process-global embedding model.
pub fn ensure_model() -> &'static EmbeddingModel {
    MODEL.get_or_init(EmbeddingModel::new)
}

/// Returns an L2-normalized 384-dimensional embedding vector.
///
/// When the BGE-small-en-v1.5 model is loaded, this produces a real semantic
/// embedding. Otherwise falls back to a deterministic SHA-256 hash projection.
pub fn embed(text: &str) -> Result<Vec<f32>, InferenceError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(InferenceError::EmptyInput);
    }

    ensure_model().embed(trimmed)
}

/// Searches the active vector table and returns page-ranked matches.
pub fn search_vec(
    query: &str,
    k: usize,
    wing_filter: Option<&str>,
    conn: &Connection,
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

    let mut sql = format!(
        "SELECT p.slug, p.title, p.summary, \
                MAX(1.0 - vec_distance_cosine(pev.embedding, ?1)) AS score, \
                p.wing \
         FROM {vec_table} pev \
         JOIN page_embeddings pe ON pev.rowid = pe.vec_rowid \
         JOIN pages p ON p.id = pe.page_id \
         WHERE pe.model = ?2"
    );

    let mut params: Vec<Box<dyn ToSql>> = vec![Box::new(query_blob), Box::new(model_name)];

    if let Some(wing) = wing_filter {
        sql.push_str(" AND p.wing = ?3");
        params.push(Box::new(wing.to_owned()));
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
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        let dir = tempfile::TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test_brain.db");
        let conn = db::open(db_path.to_str().expect("utf8 path")).expect("open db");
        std::mem::forget(dir);
        conn
    }

    #[test]
    fn embed_returns_normalized_vector_of_expected_length() {
        let embedding = embed("Alice works at Acme Corp").expect("embed text");
        let norm = embedding
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt();

        assert_eq!(embedding.len(), EMBEDDING_DIMENSIONS);
        assert!((norm - 1.0).abs() < 1e-5, "unexpected norm: {norm}");
    }

    #[test]
    fn embed_returns_error_for_empty_input() {
        let err = embed("   ").expect_err("empty input should fail");
        assert!(matches!(err, InferenceError::EmptyInput));
    }

    #[test]
    fn search_vec_on_empty_db_returns_empty_vec() {
        let conn = open_test_db();
        let results = search_vec("board member tech company", 5, None, &conn)
            .expect("empty db search should succeed");

        assert!(results.is_empty());
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
             VALUES (?1, 'bge-small-en-v1.5', 1, 'truth_section', 0, 'startup founder', 'hash', 2, 'State')",
            rusqlite::params![page_id],
        )
        .expect("insert embedding metadata");

        let results = search_vec("startup founder", 5, Some("people"), &conn)
            .expect("vector search should succeed");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].slug, "people/alice");
        assert!(
            results[0].score > 0.99,
            "unexpected score: {}",
            results[0].score
        );
    }
}
