use quaid::core::conversation::model_lifecycle::{cache_dir_for_alias, resolve_model_alias};
use quaid::core::conversation::slm::{parse_response, LazySlmRunner, SlmError, SlmRunner};
use quaid::core::types::RawFact;
use safetensors::tensor::{serialize_to_file, Dtype, TensorView};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tokenizers::models::wordlevel::WordLevel;
use tokenizers::pre_tokenizers::whitespace::Whitespace;
use tokenizers::Tokenizer;

#[test]
fn slm_runner_generates_deterministic_output_from_tiny_fixture() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");

    let mut runner = SlmRunner::load("phi-3.5-mini").expect("load tiny phi3 model");
    let output = runner.infer("hello", 1).expect("run deterministic infer");

    assert_eq!(output, "world");
}

#[test]
fn parse_response_strips_json_fence_and_preserves_fact_shape() {
    let parsed = parse_response(
        "```json\n{\"facts\":[{\"kind\":\"preference\",\"about\":\"language\",\"summary\":\"Rust is preferred\"}]}\n```",
    )
    .expect("parse fenced response");

    assert_eq!(
        parsed.facts,
        vec![RawFact::Preference {
            about: "language".to_string(),
            strength: None,
            summary: "Rust is preferred".to_string(),
        }]
    );
}

#[test]
fn lazy_runner_loads_on_first_infer_and_reuses() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");

    let runtime = LazySlmRunner::new();
    assert!(!runtime.is_loaded(), "should start unloaded");

    let first = runtime
        .infer("phi-3.5-mini", "hello", 1)
        .expect("first infer ok");
    assert_eq!(first, "world");
    assert!(runtime.is_loaded());
    assert!(!runtime.is_runtime_disabled());

    let second = runtime
        .infer("phi-3.5-mini", "hello", 0)
        .expect("second infer ok");
    assert!(second.is_empty(), "zero max_tokens must yield empty output");
    assert!(!runtime.is_runtime_disabled());
}

#[test]
fn lazy_runner_runtime_disables_after_cache_load_failure() {
    let temp = tempfile::tempdir().expect("create temp dir");
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());

    let runtime = LazySlmRunner::new();
    let error = runtime
        .infer("phi-3.5-mini", "hello", 1)
        .expect_err("cache miss must fail closed");
    assert!(matches!(error, SlmError::Cache(_)));
    assert!(runtime.is_runtime_disabled());
    assert!(!runtime.is_loaded());

    let follow_up = runtime
        .infer("phi-3.5-mini", "hello", 1)
        .expect_err("runtime must stay fail-closed");
    assert!(matches!(follow_up, SlmError::RuntimeDisabled { .. }));
}

#[test]
fn parse_response_rejects_unknown_kind_as_whole_response_error() {
    let error = parse_response(r#"{"facts":[{"kind":"unknown_kind","foo":"bar"}]}"#)
        .expect_err("unknown kind must fail");
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn parse_response_rejects_missing_required_field_as_whole_response_error() {
    let error = parse_response(r#"{"facts":[{"kind":"decision","summary":"we chose Rust"}]}"#)
        .expect_err("missing `chose` must fail");
    assert!(matches!(error, SlmError::Parse { .. }));
}

static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> &'static Mutex<()> {
    ENV_LOCK.get_or_init(|| Mutex::new(()))
}

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
    _lock: MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(key: &'static str, value: String) -> Self {
        let lock = env_lock()
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
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

fn seed_tiny_phi3_cache(alias: &str) {
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

    let embed = floats_to_bytes(&[0.0, 0.0, 10.0, 0.0, 0.0, 10.0, 0.0, 0.0]);
    let norm = floats_to_bytes(&[1.0, 1.0]);
    let lm_head = floats_to_bytes(&[0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);

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
