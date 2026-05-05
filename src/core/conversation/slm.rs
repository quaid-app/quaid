use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::phi3::{Config as Phi3Config, Model as Phi3Model};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use thiserror::Error;
use tokenizers::Tokenizer;

use crate::core::conversation::model_lifecycle::{
    load_model_from_local_cache, ModelLifecycleError,
};
use crate::core::types::{ExtractionFactValidationError, ExtractionResponse, RawFact};

const DEFAULT_SLM_SEED: u64 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlmInferenceConfig {
    pub seed: u64,
}

impl Default for SlmInferenceConfig {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SLM_SEED,
        }
    }
}

#[derive(Debug)]
pub struct SlmRunner {
    tokenizer: Tokenizer,
    model: Phi3Model,
    device: Device,
    inference: SlmInferenceConfig,
    eos_token_id: Option<u32>,
    #[allow(dead_code)]
    model_dir: PathBuf,
}

#[derive(Debug, Default)]
pub struct LazySlmRunner {
    state: Mutex<LazySlmState>,
}

#[derive(Debug, Default)]
struct LazySlmState {
    runner: Option<SlmRunner>,
    runtime_disabled: bool,
    last_error: Option<String>,
}

#[derive(Debug, Error)]
pub enum SlmError {
    #[error("input prompt is empty")]
    EmptyPrompt,

    #[error(transparent)]
    Cache(#[from] ModelLifecycleError),

    #[error("slm runtime is disabled: {message}")]
    RuntimeDisabled { message: String },

    #[error("slm config at {path} is invalid: {message}")]
    Config { path: String, message: String },

    #[error("slm tokenizer at {path} is invalid: {message}")]
    Tokenizer { path: String, message: String },

    #[error("slm weights are unavailable in {cache_dir}: {message}")]
    Weights { cache_dir: String, message: String },

    #[error("slm inference failed: {message}")]
    Inference { message: String },

    #[error("slm inference panicked: {message}")]
    Panic { message: String },

    #[error("slm output was not valid JSON: {message}")]
    Parse { message: String },
}

#[derive(Debug, Deserialize)]
struct ModelConfigEnvelope {
    model_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawExtractionEnvelope {
    facts: Vec<JsonValue>,
}

impl SlmRunner {
    pub fn load(alias: &str) -> Result<Self, SlmError> {
        Self::catch_load(|| Self::load_inner(alias))
    }

    fn load_inner(alias: &str) -> Result<Self, SlmError> {
        #[cfg(test)]
        if alias == "__panic_during_load__" {
            panic!("load panic for test");
        }

        let model_dir = load_model_from_local_cache(alias)?;
        let config_path = model_dir.join("config.json");
        let config_text = fs::read_to_string(&config_path).map_err(|error| SlmError::Config {
            path: config_path.display().to_string(),
            message: error.to_string(),
        })?;
        let envelope: ModelConfigEnvelope =
            serde_json::from_str(&config_text).map_err(|error| SlmError::Config {
                path: config_path.display().to_string(),
                message: error.to_string(),
            })?;
        match envelope.model_type.as_deref() {
            Some("phi3") => {}
            Some(other) => {
                return Err(SlmError::Config {
                    path: config_path.display().to_string(),
                    message: format!("unsupported model_type `{other}`; only phi3 is wired"),
                });
            }
            None => {
                return Err(SlmError::Config {
                    path: config_path.display().to_string(),
                    message: "missing model_type".to_string(),
                });
            }
        }
        let config: Phi3Config =
            serde_json::from_str(&config_text).map_err(|error| SlmError::Config {
                path: config_path.display().to_string(),
                message: error.to_string(),
            })?;

        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|error| SlmError::Tokenizer {
                path: tokenizer_path.display().to_string(),
                message: error.to_string(),
            })?;

        let model_paths = safetensor_paths(&model_dir)?;
        let device = Device::Cpu;
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&model_paths, DType::F32, &device) }
            .map_err(|error| SlmError::Weights {
                cache_dir: model_dir.display().to_string(),
                message: error.to_string(),
            })?;
        let model = Phi3Model::new(&config, vb).map_err(|error| SlmError::Inference {
            message: format!("build phi3 model: {error}"),
        })?;

        Ok(Self {
            tokenizer,
            model,
            device,
            inference: SlmInferenceConfig::default(),
            eos_token_id: config.eos_token_id,
            model_dir,
        })
    }

    fn catch_load<T>(operation: impl FnOnce() -> Result<T, SlmError>) -> Result<T, SlmError> {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(result) => result,
            Err(payload) => Err(SlmError::Panic {
                message: panic_payload_message(payload),
            }),
        }
    }

    pub fn infer(&mut self, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        self.catch_infer(|runner| runner.infer_inner(prompt, max_tokens))
    }

    fn infer_inner(&mut self, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        if prompt.trim().is_empty() {
            return Err(SlmError::EmptyPrompt);
        }
        if max_tokens == 0 {
            return Ok(String::new());
        }

        let encoding =
            self.tokenizer
                .encode(prompt, false)
                .map_err(|error| SlmError::Inference {
                    message: format!("tokenizer encode: {error}"),
                })?;
        let mut all_tokens = encoding.get_ids().to_vec();
        if all_tokens.is_empty() {
            return Err(SlmError::EmptyPrompt);
        }

        let mut logits = LogitsProcessor::from_sampling(self.inference.seed, Sampling::ArgMax);
        let mut generated = Vec::new();
        let mut seqlen_offset = 0usize;

        for step in 0..max_tokens {
            let context_tokens = if step == 0 {
                all_tokens.as_slice()
            } else {
                &all_tokens[all_tokens.len() - 1..]
            };
            let input = Tensor::new(context_tokens, &self.device)
                .and_then(|tensor| tensor.reshape((1, context_tokens.len())))
                .map_err(|error| SlmError::Inference {
                    message: format!("prepare input tensor: {error}"),
                })?;
            let next_logits = self
                .model
                .forward(&input, seqlen_offset)
                .and_then(|tensor| tensor.squeeze(0))
                .and_then(|tensor| tensor.squeeze(0))
                .map_err(|error| SlmError::Inference {
                    message: format!("model forward: {error}"),
                })?;
            seqlen_offset += context_tokens.len();

            let next_token = logits
                .sample(&next_logits)
                .map_err(|error| SlmError::Inference {
                    message: format!("sample logits: {error}"),
                })?;
            if Some(next_token) == self.eos_token_id {
                break;
            }
            all_tokens.push(next_token);
            generated.push(next_token);
        }

        self.tokenizer
            .decode(&generated, true)
            .map_err(|error| SlmError::Inference {
                message: format!("tokenizer decode: {error}"),
            })
    }

    fn catch_infer<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, SlmError>,
    ) -> Result<T, SlmError> {
        let result = catch_unwind(AssertUnwindSafe(|| operation(self)));
        self.model.clear_kv_cache();
        match result {
            Ok(result) => result,
            Err(payload) => Err(SlmError::Panic {
                message: panic_payload_message(payload),
            }),
        }
    }
}

impl LazySlmState {
    fn disable_runtime(&mut self, error: &SlmError) {
        self.runner = None;
        self.runtime_disabled = true;
        self.last_error = Some(error.to_string());
    }
}

impl LazySlmRunner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn infer(&self, alias: &str, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.runtime_disabled {
            return Err(SlmError::RuntimeDisabled {
                message: state
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "re-enable extraction to recover".to_string()),
            });
        }
        if state.runner.is_none() {
            match SlmRunner::load(alias) {
                Ok(runner) => state.runner = Some(runner),
                Err(error) => {
                    if should_disable_runtime_after_load_failure(&error) {
                        state.disable_runtime(&error);
                    }
                    return Err(error);
                }
            }
        }

        let result = state
            .runner
            .as_mut()
            .expect("runner inserted above")
            .infer(prompt, max_tokens);
        if let Err(error @ SlmError::Panic { .. }) = &result {
            state.disable_runtime(error);
        }
        result
    }

    pub fn is_loaded(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runner
            .is_some()
    }

    pub fn is_runtime_disabled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runtime_disabled
    }
}

pub fn parse_response(raw: &str) -> Result<ExtractionResponse, SlmError> {
    let trimmed = raw.trim();
    let json = strip_json_fence(trimmed).unwrap_or(trimmed);
    let envelope: RawExtractionEnvelope =
        serde_json::from_str(json).map_err(|error| SlmError::Parse {
            message: error.to_string(),
        })?;

    let mut facts = Vec::new();
    let mut validation_errors = Vec::new();
    for (index, value) in envelope.facts.into_iter().enumerate() {
        let raw_kind = raw_fact_kind(&value);
        match serde_json::from_value::<RawFact>(value) {
            Ok(fact) => {
                if let Some(message) = validate_fact(&fact) {
                    validation_errors.push(ExtractionFactValidationError {
                        index,
                        kind: Some(
                            match &fact {
                                RawFact::Decision { .. } => "decision",
                                RawFact::Preference { .. } => "preference",
                                RawFact::Fact { .. } => "fact",
                                RawFact::ActionItem { .. } => "action_item",
                            }
                            .to_string(),
                        ),
                        message,
                    });
                } else {
                    facts.push(fact);
                }
            }
            Err(error) => validation_errors.push(ExtractionFactValidationError {
                index,
                kind: raw_kind,
                message: error.to_string(),
            }),
        }
    }

    Ok(ExtractionResponse {
        facts,
        validation_errors,
    })
}

fn safetensor_paths(model_dir: &Path) -> Result<Vec<PathBuf>, SlmError> {
    let mut paths = fs::read_dir(model_dir)
        .map_err(|error| SlmError::Weights {
            cache_dir: model_dir.display().to_string(),
            message: error.to_string(),
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("safetensors"))
        })
        .collect::<Vec<_>>();
    paths.sort();

    if paths.is_empty() {
        return Err(SlmError::Weights {
            cache_dir: model_dir.display().to_string(),
            message: "no .safetensors weights found".to_string(),
        });
    }

    Ok(paths)
}

fn strip_json_fence(raw: &str) -> Option<&str> {
    let first_newline = raw.find('\n')?;
    let header = raw[..first_newline].trim();
    if !(header.eq_ignore_ascii_case("```json") || header == "```") {
        return None;
    }
    let body = raw[first_newline + 1..].trim_end();
    let inner = body.strip_suffix("```")?;
    Some(inner.trim())
}

fn validate_fact(fact: &RawFact) -> Option<String> {
    match fact {
        RawFact::Decision { chose, summary, .. } => validate_required_strings(
            "decision",
            &[("chose", chose.as_str()), ("summary", summary.as_str())],
        ),
        RawFact::Preference { about, summary, .. } => validate_required_strings(
            "preference",
            &[("about", about.as_str()), ("summary", summary.as_str())],
        ),
        RawFact::Fact { about, summary } => validate_required_strings(
            "fact",
            &[("about", about.as_str()), ("summary", summary.as_str())],
        ),
        RawFact::ActionItem { what, summary, .. } => validate_required_strings(
            "action_item",
            &[("what", what.as_str()), ("summary", summary.as_str())],
        ),
    }
}

fn validate_required_strings(kind: &str, fields: &[(&str, &str)]) -> Option<String> {
    fields.iter().find_map(|(field, value)| {
        if value.trim().is_empty() {
            Some(format!("{kind} facts require non-empty `{field}`"))
        } else {
            None
        }
    })
}

fn raw_fact_kind(value: &JsonValue) -> Option<String> {
    value
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::to_owned)
}

fn panic_payload_message(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(payload) => match payload.downcast::<&'static str>() {
            Ok(message) => (*message).to_string(),
            Err(_) => "non-string panic payload".to_string(),
        },
    }
}

fn should_disable_runtime_after_load_failure(error: &SlmError) -> bool {
    matches!(
        error,
        SlmError::Cache(_)
            | SlmError::Config { .. }
            | SlmError::Tokenizer { .. }
            | SlmError::Weights { .. }
            | SlmError::Inference { .. }
            | SlmError::Panic { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_response, LazySlmRunner, SlmError, SlmRunner};
    use crate::core::conversation::model_lifecycle::{cache_dir_for_alias, resolve_model_alias};
    use safetensors::tensor::{serialize_to_file, Dtype, TensorView};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;
    use tokenizers::models::wordlevel::WordLevel;
    use tokenizers::pre_tokenizers::whitespace::Whitespace;
    use tokenizers::Tokenizer;

    #[test]
    fn parse_response_accepts_json_code_fence() {
        let parsed = parse_response("```json\n{\"facts\":[]}\n```").unwrap();
        assert!(parsed.facts.is_empty());
    }

    #[test]
    fn parse_response_rejects_leading_commentary() {
        let error = parse_response("Sure:\n{\"facts\":[]}").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn infer_returns_typed_panic_error() {
        let temp = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
        seed_tiny_phi3_cache("phi-3.5-mini");

        let mut runner = SlmRunner::load("phi-3.5-mini").unwrap();
        let error = runner
            .catch_infer::<String>(|_| panic!("boom"))
            .unwrap_err();

        assert!(matches!(error, SlmError::Panic { .. }));
        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn lazy_runner_reuses_loaded_model_after_cache_is_removed() {
        let temp = tempfile::tempdir().unwrap();
        let mut guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
        seed_tiny_phi3_cache("phi-3.5-mini");

        let runtime = LazySlmRunner::new();
        let first = runtime.infer("phi-3.5-mini", "hello", 1).unwrap();
        assert_eq!(first, "world");
        assert!(runtime.is_loaded());

        let empty_cache = tempfile::tempdir().unwrap();
        guard.replace(empty_cache.path().display().to_string());

        let second = runtime.infer("phi-3.5-mini", "hello", 1).unwrap();
        assert_eq!(second, "world");
        assert!(!runtime.is_runtime_disabled());
    }

    #[test]
    fn lazy_runner_runtime_disables_after_cache_load_failure() {
        let temp = tempfile::tempdir().unwrap();
        let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());

        let runtime = LazySlmRunner::new();
        let error = runtime.infer("phi-3.5-mini", "hello", 1).unwrap_err();

        assert!(matches!(error, SlmError::Cache(_)));
        assert!(runtime.is_runtime_disabled());
        assert!(!runtime.is_loaded());

        let follow_up = runtime.infer("phi-3.5-mini", "hello", 1).unwrap_err();
        assert!(matches!(follow_up, SlmError::RuntimeDisabled { .. }));
    }

    #[test]
    fn lazy_runner_runtime_disables_after_load_panic() {
        let runtime = LazySlmRunner::new();
        let error = runtime
            .infer("__panic_during_load__", "hello", 1)
            .unwrap_err();

        assert!(matches!(error, SlmError::Panic { .. }));
        assert!(error.to_string().contains("load panic for test"));
        assert!(runtime.is_runtime_disabled());
        assert!(!runtime.is_loaded());

        let follow_up = runtime.infer("phi-3.5-mini", "hello", 1).unwrap_err();
        assert!(matches!(follow_up, SlmError::RuntimeDisabled { .. }));
        assert!(follow_up.to_string().contains("load panic for test"));
    }

    // Serialize all tests that mutate QUAID_MODEL_CACHE_DIR. Without this lock the two
    // env-var-touching tests race against each other: one test's seed writes to the
    // directory chosen by the other test's guard, causing manifest-not-found failures.
    static ENV_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    struct EnvGuard {
        key: &'static str,
        previous: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: String) -> Self {
            let lock = ENV_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self {
                key,
                previous,
                _lock: lock,
            }
        }

        fn replace(&mut self, value: String) {
            std::env::set_var(self.key, value);
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.take() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
            // _lock drops here: env var is fully restored before the lock is released
        }
    }

    pub(super) fn seed_tiny_phi3_cache(alias: &str) {
        let resolved = resolve_model_alias(alias).unwrap();
        let cache_dir = cache_dir_for_alias(alias).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();

        std::fs::write(
            cache_dir.join("config.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "model_type": "phi3",
                "vocab_size": 4,
                "hidden_act": "relu",
                "hidden_size": 2,
                "intermediate_size": 2,
                "num_hidden_layers": 0,
                "num_attention_heads": 1,
                "num_key_value_heads": 1,
                "rms_norm_eps": 1e-6,
                "rope_theta": 10_000.0,
                "bos_token_id": serde_json::Value::Null,
                "eos_token_id": 3,
                "rope_scaling": serde_json::Value::Null,
                "max_position_embeddings": 16
            }))
            .unwrap(),
        )
        .unwrap();

        let mut tokenizer = Tokenizer::new(
            WordLevel::builder()
                .vocab(HashMap::from([
                    ("<unk>".to_string(), 0),
                    ("hello".to_string(), 1),
                    ("world".to_string(), 2),
                    ("<eos>".to_string(), 3),
                ]))
                .unk_token("<unk>".to_string())
                .build()
                .unwrap(),
        );
        tokenizer.with_pre_tokenizer(Some(Whitespace));
        tokenizer
            .save(cache_dir.join("tokenizer.json"), false)
            .unwrap();

        let embed = floats_to_bytes(&[
            0.0, 0.0, // <unk>
            10.0, 0.0, // hello
            0.0, 10.0, // world
            0.0, 0.0, // eos
        ]);
        let norm = floats_to_bytes(&[1.0, 1.0]);
        let lm_head = floats_to_bytes(&[
            0.0, 0.0, // <unk>
            0.0, 0.0, // hello
            1.0, 0.0, // world
            0.0, 1.0, // eos
        ]);

        let embed_view = TensorView::new(Dtype::F32, vec![4, 2], &embed).unwrap();
        let norm_view = TensorView::new(Dtype::F32, vec![2], &norm).unwrap();
        let lm_head_view = TensorView::new(Dtype::F32, vec![4, 2], &lm_head).unwrap();
        serialize_to_file(
            [
                ("model.embed_tokens.weight", embed_view),
                ("model.norm.weight", norm_view),
                ("lm_head.weight", lm_head_view),
            ],
            &None,
            &cache_dir.join("model.safetensors"),
        )
        .unwrap();

        let manifest = serde_json::json!({
            "requested_alias": resolved.requested_alias,
            "repo_id": resolved.repo_id,
            "revision": resolved.revision,
            "files": [
                manifest_entry("config.json", &cache_dir.join("config.json")),
                manifest_entry("model.safetensors", &cache_dir.join("model.safetensors")),
                manifest_entry("tokenizer.json", &cache_dir.join("tokenizer.json")),
            ]
        });
        std::fs::write(
            cache_dir.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
    }

    fn manifest_entry(path: &str, full_path: &std::path::Path) -> serde_json::Value {
        let bytes = std::fs::read(full_path).unwrap();
        serde_json::json!({
            "path": path,
            "sha256": format!("{:x}", Sha256::digest(bytes)),
            "verified_from_source": false
        })
    }

    fn floats_to_bytes(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }
}
