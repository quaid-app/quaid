#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Fence-recovery matrix for `parse_response`, plus behavioral coverage
//! of the SLM runner's EOS-union stop set, half-precision weight load,
//! and the `LazySlmRunner` idle-unload-then-reload cycle. The
//! cache-touching tests use the same tiny hand-built Phi-3 fixture the
//! other SLM integration suites use, so no real multi-GB download is
//! required.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use quaid::core::conversation::model_lifecycle::{cache_dir_for_alias, resolve_model_alias};
use quaid::core::conversation::slm::{parse_response, LazySlmRunner, SlmError, SlmRunner};
use safetensors::tensor::{serialize_to_file, Dtype, TensorView};
use sha2::{Digest, Sha256};
use tokenizers::models::wordlevel::WordLevel;
use tokenizers::pre_tokenizers::whitespace::Whitespace;
use tokenizers::Tokenizer;

// ---------------------------------------------------------------------------
// Fence recovery: a single fenced JSON object is accepted; anything else
// (multiple objects, missing close, garbage, fence buried in prose) fails
// closed.
// ---------------------------------------------------------------------------

#[test]
fn accepts_single_json_fence_with_info_string() {
    let parsed = parse_response("```json\n{\"facts\":[]}\n```").unwrap();
    assert!(parsed.facts.is_empty());
    assert!(parsed.validation_errors.is_empty());
}

#[test]
fn accepts_bare_triple_backtick_fence() {
    let parsed = parse_response("```\n{\"facts\":[]}\n```").unwrap();
    assert!(parsed.facts.is_empty());
}

#[test]
fn accepts_tilde_fence() {
    let parsed = parse_response("~~~json\n{\"facts\":[]}\n~~~").unwrap();
    assert!(parsed.facts.is_empty());
}

#[test]
fn accepts_single_fence_with_nonempty_facts() {
    let parsed = parse_response(concat!(
        "```json\n",
        "{\"facts\":[{\"kind\":\"fact\",\"about\":\"repo\",\"summary\":\"Quaid is local-first\"}]}\n",
        "```"
    ))
    .unwrap();
    assert_eq!(parsed.facts.len(), 1);
    assert!(parsed.validation_errors.is_empty());
}

#[test]
fn accepts_fence_with_surrounding_whitespace() {
    let parsed = parse_response("\n  ```json\n{\"facts\":[]}\n```  \n").unwrap();
    assert!(parsed.facts.is_empty());
}

#[test]
fn rejects_two_fenced_objects() {
    let error = parse_response(concat!(
        "```json\n{\"facts\":[]}\n```\n",
        "```json\n{\"facts\":[]}\n```"
    ))
    .unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_fence_with_trailing_prose() {
    // A closing fence followed by commentary is not a lone fenced
    // envelope, so recovery falls through and fails closed.
    let error = parse_response("```json\n{\"facts\":[]}\n```\nHope that helps!").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_fence_with_leading_prose() {
    let error = parse_response("Here you go:\n```json\n{\"facts\":[]}\n```").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_unclosed_fence() {
    let error = parse_response("```json\n{\"facts\":[]}").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_object_without_closing_fence_but_with_opening() {
    let error = parse_response("```json\n{\"facts\":[]}\nstill talking").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_pure_garbage() {
    let error = parse_response("not json at all, just prose").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

#[test]
fn rejects_empty_input() {
    let error = parse_response("   ").unwrap_err();
    assert!(matches!(error, SlmError::Parse { .. }));
}

// ---------------------------------------------------------------------------
// EOS stop set: ids from generation_config.json are unioned with config.json.
// ---------------------------------------------------------------------------

#[test]
fn generation_config_eos_id_halts_generation() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");
    // The tiny model greedily emits token 2 ("world") for input "hello".
    // config.json's eos is token 3, so without the generation_config
    // union the model would emit "world". Declaring token 2 as an eos
    // in generation_config.json must halt generation immediately.
    write_generation_config(
        "phi-3.5-mini",
        serde_json::json!({ "eos_token_id": [2, 99] }),
    );

    let mut runner = SlmRunner::load("phi-3.5-mini").expect("load tiny phi3 model");
    let output = runner.infer("hello", 4).expect("run infer");

    assert!(
        output.is_empty(),
        "generation should stop on the unioned eos id 2, got {output:?}"
    );
}

#[test]
fn scalar_generation_config_eos_id_halts_generation() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");
    write_generation_config("phi-3.5-mini", serde_json::json!({ "eos_token_id": 2 }));

    let mut runner = SlmRunner::load("phi-3.5-mini").expect("load tiny phi3 model");
    let output = runner.infer("hello", 4).expect("run infer");

    assert!(output.is_empty(), "scalar eos union should stop on 2");
}

#[test]
fn missing_generation_config_falls_back_to_config_eos() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");
    // No generation_config.json written: only config.json eos=3 applies,
    // so the model still emits "world" (token 2).

    let mut runner = SlmRunner::load("phi-3.5-mini").expect("load tiny phi3 model");
    let output = runner.infer("hello", 1).expect("run infer");

    assert_eq!(output, "world");
}

#[test]
fn malformed_generation_config_is_non_fatal() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");
    let cache_dir = cache_dir_for_alias("phi-3.5-mini").unwrap();
    std::fs::write(cache_dir.join("generation_config.json"), b"{ not json").unwrap();

    let mut runner = SlmRunner::load("phi-3.5-mini").expect("load despite bad generation config");
    let output = runner.infer("hello", 1).expect("run infer");

    assert_eq!(output, "world");
}

// ---------------------------------------------------------------------------
// Half-precision load: F16 weights (the CPU half-precision choice, since
// candle's CPU matmul rejects BF16) still produce the expected argmax token.
// ---------------------------------------------------------------------------

#[test]
fn half_precision_weight_load_preserves_deterministic_argmax() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");

    // The runner now loads weights in half precision; the tiny fixture's
    // exact values (10.0, 1.0) are representable in F16 so the greedy
    // decode is unchanged. This guards that the dtype switch did not
    // perturb the selected token.
    let mut runner =
        SlmRunner::load("phi-3.5-mini").expect("load tiny phi3 model in half precision");
    let output = runner.infer("hello", 1).expect("run infer");

    assert_eq!(output, "world");
}

// ---------------------------------------------------------------------------
// Idle-unload: the lazy runner drops its model after the idle TTL and
// transparently reloads on the next infer.
// ---------------------------------------------------------------------------

#[test]
fn unload_if_idle_drops_then_reloads() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");

    let runtime = LazySlmRunner::new();
    assert!(!runtime.is_loaded());

    let first = runtime
        .infer("phi-3.5-mini", "hello", 1)
        .expect("first infer");
    assert_eq!(first, "world");
    assert!(runtime.is_loaded());

    // A zero idle TTL means "any idle time counts": the model is dropped.
    assert!(
        runtime.unload_if_idle(Duration::from_secs(0)),
        "idle model should unload"
    );
    assert!(
        !runtime.is_loaded(),
        "runner should be dropped after unload"
    );

    // Next infer reloads transparently and still works.
    let second = runtime
        .infer("phi-3.5-mini", "hello", 1)
        .expect("reload after unload");
    assert_eq!(second, "world");
    assert!(runtime.is_loaded());
    assert!(!runtime.is_runtime_disabled());
}

#[test]
fn unload_if_idle_keeps_recently_used_model() {
    let temp = tempfile::tempdir().unwrap();
    let _guard = EnvGuard::set("QUAID_MODEL_CACHE_DIR", temp.path().display().to_string());
    seed_tiny_phi3_cache("phi-3.5-mini");

    let runtime = LazySlmRunner::new();
    runtime.infer("phi-3.5-mini", "hello", 1).expect("infer");
    assert!(runtime.is_loaded());

    // A long TTL just after use must not drop the model.
    assert!(
        !runtime.unload_if_idle(Duration::from_secs(3600)),
        "freshly used model must not be unloaded"
    );
    assert!(runtime.is_loaded());
}

#[test]
fn unload_if_idle_on_empty_handle_is_noop() {
    let runtime = LazySlmRunner::new();
    assert!(!runtime.unload_if_idle(Duration::from_secs(0)));
    assert!(!runtime.is_loaded());
}

// ---------------------------------------------------------------------------
// Test fixtures
// ---------------------------------------------------------------------------

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

fn write_generation_config(alias: &str, body: serde_json::Value) {
    // Written after seeding and intentionally left out of the manifest;
    // manifest validation only walks recorded files, so an extra sibling
    // is tolerated and exercises the real load-time read path.
    let cache_dir = cache_dir_for_alias(alias).unwrap();
    std::fs::write(
        cache_dir.join("generation_config.json"),
        serde_json::to_vec_pretty(&body).unwrap(),
    )
    .unwrap();
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
