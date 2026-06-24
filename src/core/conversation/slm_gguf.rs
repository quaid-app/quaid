//! In-process GGUF (q4_K_M) SLM runner for the Qwen3 extraction model, loaded
//! via candle's `quantized_qwen3`. Mirrors the Phi-3 [`super::slm::SlmRunner`]
//! infer loop — greedy `Sampling::ArgMax`, per-call KV-cache clear, and
//! `catch_unwind` panic isolation — behind the same [`SlmError`] type, so
//! `LazySlmRunner` can select it for a cached `.gguf` model with no changes to
//! the `SlmClient` consumers.
//!
//! The GGUF carries Qwen3's architecture in its `general.architecture`
//! metadata (there is no HF `config.json`), and its EOS ids come from
//! `tokenizer.ggml.eos_token_id` plus the `<|im_end|>` chat-template marker.
//! The HF tokenizer is loaded from a sibling `tokenizer.json` provisioned
//! alongside the weights (candle has no GGUF→`tokenizers::Tokenizer` helper;
//! reconstructing it from GGUF metadata is a follow-up).

use std::collections::HashSet;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use candle_core::quantized::gguf_file;
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::quantized_qwen3::ModelWeights;
use tokenizers::Tokenizer;

use super::model_lifecycle::load_model_from_local_cache;
use super::slm::{panic_payload_message, SlmError};

/// Greedy decode seed (consulted only for `ArgMax` tie-breaks).
const DEFAULT_SLM_SEED: u64 = 0;

/// Hard cap on prompt + generated tokens. The model's 262K trained context
/// would size the KV cache into multiple GB; 8K bounds it (qwen3 §D5/3.4).
const QWEN3_MAX_CONTEXT: usize = 8192;

/// Loaded GGUF SLM ready for greedy inference.
pub struct SlmGgufRunner {
    tokenizer: Tokenizer,
    model: ModelWeights,
    device: Device,
    eos_token_ids: HashSet<u32>,
    /// Held for diagnostics (surfaced via the `Debug` impl).
    model_dir: PathBuf,
}

impl std::fmt::Debug for SlmGgufRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SlmGgufRunner")
            .field("model_dir", &self.model_dir)
            .field("eos_token_ids", &self.eos_token_ids)
            .finish()
    }
}

/// Returns the single `.gguf` file in `model_dir`, if exactly one is present.
pub fn find_gguf_file(model_dir: &Path) -> Option<PathBuf> {
    let mut found = None;
    for entry in std::fs::read_dir(model_dir).ok()?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("gguf") {
            if found.is_some() {
                return None; // ambiguous: more than one .gguf
            }
            found = Some(path);
        }
    }
    found
}

impl SlmGgufRunner {
    /// Load the GGUF model identified by `alias` from its local cache. Panics
    /// during load are caught and surfaced as [`SlmError::Panic`].
    pub fn load(alias: &str) -> Result<Self, SlmError> {
        match catch_unwind(AssertUnwindSafe(|| Self::load_inner(alias))) {
            Ok(result) => result,
            Err(payload) => Err(SlmError::Panic {
                message: panic_payload_message(payload),
            }),
        }
    }

    fn load_inner(alias: &str) -> Result<Self, SlmError> {
        let model_dir = load_model_from_local_cache(alias)?;
        let gguf_path = find_gguf_file(&model_dir).ok_or_else(|| SlmError::Weights {
            cache_dir: model_dir.display().to_string(),
            message: "expected exactly one .gguf weight file".to_owned(),
        })?;

        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|error| SlmError::Tokenizer {
                path: tokenizer_path.display().to_string(),
                message: error.to_string(),
            })?;

        let device = Device::Cpu;
        let mut file = std::fs::File::open(&gguf_path).map_err(|error| SlmError::Weights {
            cache_dir: model_dir.display().to_string(),
            message: format!("open {}: {error}", gguf_path.display()),
        })?;
        let content =
            gguf_file::Content::read(&mut file).map_err(|error| SlmError::Weights {
                cache_dir: model_dir.display().to_string(),
                message: format!("read gguf {}: {error}", gguf_path.display()),
            })?;

        let eos_token_ids = collect_eos_token_ids(&content, &tokenizer);

        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(|error| {
            SlmError::Inference {
                message: format!("build qwen3 gguf model: {error}"),
            }
        })?;

        Ok(Self {
            tokenizer,
            model,
            device,
            eos_token_ids,
            model_dir,
        })
    }

    /// Run greedy (argmax) inference over `prompt` for up to `max_tokens`
    /// newly generated tokens, clearing the KV cache afterward so the runner
    /// is reusable. Panics are caught and surfaced as [`SlmError::Panic`].
    pub fn infer(&mut self, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        let result = catch_unwind(AssertUnwindSafe(|| self.infer_inner(prompt, max_tokens)));
        self.model.clear_kv_cache();
        match result {
            Ok(inner) => inner,
            Err(payload) => Err(SlmError::Panic {
                message: panic_payload_message(payload),
            }),
        }
    }

    fn infer_inner(&mut self, prompt: &str, max_tokens: usize) -> Result<String, SlmError> {
        if prompt.trim().is_empty() {
            return Err(SlmError::EmptyPrompt);
        }
        if max_tokens == 0 {
            return Ok(String::new());
        }

        let encoding = self
            .tokenizer
            .encode(prompt, true)
            .map_err(|error| SlmError::Inference {
                message: format!("tokenizer encode: {error}"),
            })?;
        let mut all_tokens = encoding.get_ids().to_vec();
        if all_tokens.is_empty() {
            return Err(SlmError::EmptyPrompt);
        }
        // Bound prompt + generation to the KV-cache cap, leaving room for at
        // least one generated token.
        if all_tokens.len() >= QWEN3_MAX_CONTEXT {
            let start = all_tokens.len() - (QWEN3_MAX_CONTEXT - 1);
            all_tokens.drain(..start);
        }
        let generation_budget = max_tokens.min(QWEN3_MAX_CONTEXT - all_tokens.len());

        let mut logits = LogitsProcessor::from_sampling(DEFAULT_SLM_SEED, Sampling::ArgMax);
        let mut generated = Vec::new();
        let mut seqlen_offset = 0usize;

        for step in 0..generation_budget {
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
                .map_err(|error| SlmError::Inference {
                    message: format!("model forward: {error}"),
                })?;
            seqlen_offset += context_tokens.len();

            let next_token = logits
                .sample(&next_logits)
                .map_err(|error| SlmError::Inference {
                    message: format!("sample logits: {error}"),
                })?;
            if self.eos_token_ids.contains(&next_token) {
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
}

/// Build the stop-token set from the GGUF's `tokenizer.ggml.eos_token_id`
/// metadata plus the `<|im_end|>` chat-template marker resolved through the
/// tokenizer, so generation halts at the assistant-turn boundary.
fn collect_eos_token_ids(content: &gguf_file::Content, tokenizer: &Tokenizer) -> HashSet<u32> {
    let mut ids = HashSet::new();
    if let Some(value) = content.metadata.get("tokenizer.ggml.eos_token_id") {
        if let Ok(id) = value.to_u32() {
            ids.insert(id);
        }
    }
    if let Some(id) = tokenizer.token_to_id("<|im_end|>") {
        ids.insert(id);
    }
    ids
}
