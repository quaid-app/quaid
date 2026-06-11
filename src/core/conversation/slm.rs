#![expect(
    clippy::expect_used,
    reason = "addressed in remove-production-panic-paths"
)]

//! Small language model runner for fact extraction.
//!
//! Wraps a candle-backed Phi-3 model: loads the tokenizer and
//! weights from the local cache, performs greedy (argmax) inference
//! on a prompt, and parses the JSON envelope the model emits into
//! typed `crate::core::types::RawFact` values. All `load` and
//! `infer` calls are wrapped in `catch_unwind` so a model-side panic
//! becomes a typed `SlmError::Panic` instead of aborting the
//! process. `LazySlmRunner` provides a lazy, reusable handle that
//! disables the runtime after fatal load failures so the worker
//! keeps draining the queue with explicit errors instead of looping
//! on the same failure.
//!
//! See also: `super::model_lifecycle` for the cache-resolution
//! layer feeding `SlmRunner::load`, and `super::extractor` for the
//! caller that drives prompt construction and persists the parsed
//! response.

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

/// Knobs that control SLM token sampling; held on the runner so each
/// inference call uses a stable, reproducible setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlmInferenceConfig {
    /// PRNG seed handed to the logits processor; with the current
    /// `Sampling::ArgMax` policy it is only consulted for tie-breaks.
    pub seed: u64,
}

impl Default for SlmInferenceConfig {
    fn default() -> Self {
        Self {
            seed: DEFAULT_SLM_SEED,
        }
    }
}

/// Loaded SLM ready to run inference; owns the tokenizer, weights,
/// and the candle device the model was built against.
#[derive(Debug)]
pub struct SlmRunner {
    tokenizer: Tokenizer,
    model: Phi3Model,
    device: Device,
    inference: SlmInferenceConfig,
    eos_token_id: Option<u32>,
    #[expect(
        dead_code,
        reason = "model_dir is held for diagnostics/Debug logging of which weights backed this runner; not read from struct fields directly"
    )]
    model_dir: PathBuf,
}

/// Reusable lazy handle around an [`SlmRunner`]: loads the model on
/// first use, caches it for subsequent calls, and disables the
/// runtime after fatal failures so the worker reports a typed error
/// instead of retrying a doomed load.
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

/// Errors surfaced by SLM load, inference, and response parsing.
#[derive(Debug, Error)]
pub enum SlmError {
    /// Caller passed a blank or whitespace-only prompt.
    #[error("input prompt is empty")]
    EmptyPrompt,

    /// Resolving or downloading the model cache failed.
    #[error(transparent)]
    Cache(#[from] ModelLifecycleError),

    /// A prior fatal failure flipped the lazy runtime into a disabled
    /// state; further calls return this variant until re-enabled.
    #[error("slm runtime is disabled: {message}")]
    RuntimeDisabled {
        /// Human-readable message describing the original failure
        /// that disabled the runtime.
        message: String,
    },

    /// `config.json` for the model was missing, unreadable, or
    /// declared an unsupported model type.
    #[error("slm config at {path} is invalid: {message}")]
    Config {
        /// Filesystem path of the offending config file.
        path: String,
        /// Underlying parser or validation message.
        message: String,
    },

    /// `tokenizer.json` for the model could not be loaded or parsed.
    #[error("slm tokenizer at {path} is invalid: {message}")]
    Tokenizer {
        /// Filesystem path of the tokenizer file.
        path: String,
        /// Underlying parser or validation message.
        message: String,
    },

    /// Required `.safetensors` weights were missing or unreadable.
    #[error("slm weights are unavailable in {cache_dir}: {message}")]
    Weights {
        /// Cache directory that was inspected.
        cache_dir: String,
        /// Underlying I/O or load message.
        message: String,
    },

    /// Inference failed deterministically inside candle (e.g. tensor
    /// shape mismatch, tokenizer encode/decode error).
    #[error("slm inference failed: {message}")]
    Inference {
        /// Underlying inference-stage message.
        message: String,
    },

    /// `load` or `infer` panicked; converted from `catch_unwind`.
    #[error("slm inference panicked: {message}")]
    Panic {
        /// Best-effort string form of the panic payload.
        message: String,
    },

    /// The model emitted output that did not parse as the expected
    /// `{ "facts": [...] }` JSON envelope.
    #[error("slm output was not valid JSON: {message}")]
    Parse {
        /// Underlying JSON parse-error message.
        message: String,
    },
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
    /// Load the model identified by `alias` from its local cache,
    /// returning a runner ready for [`infer`](Self::infer); panics
    /// during load are caught and surfaced as
    /// [`SlmError::Panic`].
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
        let config = parse_phi3_config(&config_text).map_err(|error| SlmError::Config {
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
        #[expect(
            unsafe_code,
            reason = "candle's VarBuilder::from_mmaped_safetensors mmaps the on-disk Phi-3 weights; safety holds because we treat the cached model files as immutable for the lifetime of the VarBuilder"
        )]
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

    /// Run greedy (argmax) inference over `prompt` for up to
    /// `max_tokens` newly generated tokens and return the decoded
    /// completion; the KV cache is cleared on every call so the
    /// runner is reusable.
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
    /// Construct an empty handle; no model is loaded until the first
    /// [`infer`](Self::infer) call.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load the model on first use and run inference, caching the
    /// runner for subsequent calls; fatal load or inference failures
    /// disable the runtime so the next call returns
    /// [`SlmError::RuntimeDisabled`] immediately.
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

    /// Report whether a runner has been successfully loaded into the
    /// handle; useful for diagnostics and tests.
    pub fn is_loaded(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runner
            .is_some()
    }

    /// Report whether the runtime has been disabled after a fatal
    /// failure and is short-circuiting subsequent calls.
    pub fn is_runtime_disabled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .runtime_disabled
    }
}

fn parse_phi3_config(config_text: &str) -> serde_json::Result<Phi3Config> {
    let mut config_value: JsonValue = serde_json::from_str(config_text)?;
    normalize_phi3_rope_scaling(&mut config_value);
    serde_json::from_value(config_value)
}

fn normalize_phi3_rope_scaling(config: &mut JsonValue) {
    let Some(rope_scaling) = config.get_mut("rope_scaling") else {
        return;
    };
    let JsonValue::Object(rope_scaling_object) = rope_scaling else {
        return;
    };
    let normalized = rope_scaling_object
        .get("type")
        .and_then(JsonValue::as_str)
        .map_or(JsonValue::Null, |rope_type| {
            JsonValue::String(rope_type.to_string())
        });
    *rope_scaling = normalized;
}

/// Parse the model's raw output into the typed
/// [`ExtractionResponse`] envelope, accepting a bare JSON object or a
/// plain-prose wrapper around exactly one valid JSON object. Ordinary
/// prose punctuation like parentheses or brackets is allowed, but
/// structural wrappers outside the envelope are rejected so recovery
/// fails closed.
/// Per-fact validation errors are collected rather than aborting the
/// whole response.
pub fn parse_response(raw: &str) -> Result<ExtractionResponse, SlmError> {
    let trimmed = raw.trim();
    let envelope = parse_raw_envelope(trimmed)?;

    let mut facts = Vec::new();
    let mut validation_errors = Vec::new();
    for (index, value) in envelope.facts.into_iter().enumerate() {
        let raw_kind = raw_fact_kind(&value);
        match serde_json::from_value::<RawFact>(value) {
            Ok(fact) => {
                if let Some(message) = validate_raw_fact(&fact) {
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

fn parse_raw_envelope(raw: &str) -> Result<RawExtractionEnvelope, SlmError> {
    match serde_json::from_str(raw) {
        Ok(envelope) => Ok(envelope),
        Err(primary_error) => {
            let Some(candidate) = recover_commentary_wrapped_object(raw) else {
                return Err(SlmError::Parse {
                    message: primary_error.to_string(),
                });
            };

            serde_json::from_str(candidate).map_err(|error| SlmError::Parse {
                message: error.to_string(),
            })
        }
    }
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

fn recover_commentary_wrapped_object(raw: &str) -> Option<&str> {
    let candidates = extract_top_level_json_object_spans(raw);
    let [candidate] = candidates.as_slice() else {
        return None;
    };
    let prefix = &raw[..candidate.0];
    let suffix = &raw[candidate.1..];
    if !candidate_has_adjacent_container_chars(prefix, suffix)
        && !candidate_is_inside_container(prefix, suffix)
        && wrapper_is_plain_commentary(prefix)
        && wrapper_is_plain_commentary(suffix)
    {
        Some(&raw[candidate.0..candidate.1])
    } else {
        None
    }
}

fn candidate_has_adjacent_container_chars(prefix: &str, suffix: &str) -> bool {
    matches!(
        prefix.chars().rev().find(|ch| !ch.is_whitespace()),
        Some('(' | '[' | '"' | '\'')
    ) || matches!(
        suffix.chars().find(|ch| !ch.is_whitespace()),
        Some(')' | ']' | '"' | '\'')
    )
}

fn candidate_is_inside_container(prefix: &str, suffix: &str) -> bool {
    candidate_is_inside_bracket_or_parenthesis_container(prefix)
        || candidate_is_inside_quote_container(prefix, suffix)
}

fn candidate_is_inside_bracket_or_parenthesis_container(prefix: &str) -> bool {
    let mut stack = Vec::new();

    for ch in prefix.chars() {
        match ch {
            '(' | '[' => stack.push(ch),
            ')' => {
                if matches!(stack.last(), Some('(')) {
                    stack.pop();
                }
            }
            ']' => {
                if matches!(stack.last(), Some('[')) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }

    !stack.is_empty()
}

fn candidate_is_inside_quote_container(prefix: &str, _suffix: &str) -> bool {
    unmatched_quote_delimiter(prefix).is_some()
}

fn unmatched_quote_delimiter(text: &str) -> Option<char> {
    let chars = text.chars().collect::<Vec<_>>();
    let mut active_quote = None;
    let mut escape = false;

    for (index, ch) in chars.iter().copied().enumerate() {
        match active_quote {
            Some('"') => {
                if escape {
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == '"' {
                    active_quote = None;
                }
            }
            Some('\'') => {
                if ch == '\'' && single_quote_is_delimiter(&chars, index) {
                    active_quote = None;
                }
            }
            None => match ch {
                '"' => active_quote = Some('"'),
                '\'' if single_quote_is_delimiter(&chars, index) => active_quote = Some('\''),
                _ => {}
            },
            Some(_) => {}
        }
    }

    active_quote
}

fn single_quote_is_delimiter(chars: &[char], index: usize) -> bool {
    let prev = index
        .checked_sub(1)
        .and_then(|prev| chars.get(prev))
        .copied();
    let next = chars.get(index + 1).copied();

    !prev
        .zip(next)
        .is_some_and(|(prev, next)| prev.is_ascii_alphanumeric() && next.is_ascii_alphanumeric())
}

fn wrapper_is_plain_commentary(wrapper: &str) -> bool {
    wrapper
        .trim()
        .split('\n')
        .all(commentary_line_is_plain_text)
}

fn commentary_line_is_plain_text(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }
    if line_is_structural_wrapper(trimmed) || line_is_bracket_or_parenthesis_wrapper(trimmed) {
        return false;
    }
    if line_is_prompt_echo_label(trimmed) || line_is_prompt_echo_sentence(trimmed) {
        return false;
    }

    let chars = trimmed.chars().collect::<Vec<_>>();
    chars.iter().enumerate().all(|(index, ch)| match ch {
        '{' | '}' | '<' | '>' | '`' => false,
        ':' => !colon_looks_structural(&chars, index),
        _ => true,
    })
}

fn line_is_structural_wrapper(line: &str) -> bool {
    line.starts_with("```")
        || line.starts_with("~~~")
        || line.starts_with('<')
        || line.ends_with('>')
        || line_starts_with_list_marker(line)
}

fn line_is_bracket_or_parenthesis_wrapper(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if compact.is_empty() {
        return false;
    }

    compact
        .chars()
        .all(|ch| matches!(ch, '[' | ']' | '(' | ')'))
        || fully_wrapped_line_is_structural(&compact)
}

fn line_is_prompt_echo_label(line: &str) -> bool {
    let normalized = normalize_prompt_echo_line(line.trim_end_matches(':'));

    matches!(
        normalized.as_str(),
        "example"
            | "examples"
            | "schema"
            | "json schema"
            | "response schema"
            | "output schema"
            | "allowed output"
            | "allowed outputs"
            | "allowed output only"
            | "allowed outputs only"
            | "allowed response"
            | "allowed responses"
            | "allowed response only"
            | "allowed responses only"
            | "response format"
            | "output format"
            | "return"
            | "return only"
    )
}

fn line_is_prompt_echo_sentence(line: &str) -> bool {
    matches!(
        normalize_prompt_echo_line(line).as_str(),
        "you are not a chat partner return exactly one json object and nothing else"
            | "skip ephemeral content greetings clarifications transient task state"
            | "skip facts you already extracted in prior windows"
            | "facts must be supported by the windowed turns do not infer beyond what was said"
            | "required kind summary plus the type specific structured field s"
            | "return facts empty array if nothing durable"
    )
}

fn normalize_prompt_echo_line(line: &str) -> String {
    let normalized = line
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();

    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fully_wrapped_line_is_structural(line: &str) -> bool {
    let inner = if (line.starts_with('[') && line.ends_with(']'))
        || (line.starts_with('(') && line.ends_with(')'))
    {
        &line[1..line.len() - 1]
    } else {
        return false;
    };

    let inner = inner.trim();
    inner.is_empty()
        || !inner.chars().any(char::is_alphanumeric)
        || inner.chars().any(|ch| {
            matches!(
                ch,
                '{' | '}' | '[' | ']' | '(' | ')' | '<' | '>' | '`' | '"'
            )
        })
        || line_starts_with_list_marker(inner)
}

fn line_starts_with_list_marker(line: &str) -> bool {
    line_starts_with_unordered_list_marker(line) || line_starts_with_ordered_list_marker(line)
}

fn line_starts_with_unordered_list_marker(line: &str) -> bool {
    matches!(line.chars().next(), Some('-' | '*' | '+'))
        && line[1..].chars().next().is_none_or(char::is_whitespace)
}

fn line_starts_with_ordered_list_marker(line: &str) -> bool {
    let digit_count = line.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return false;
    }

    let Some(rest) = line.get(digit_count..) else {
        return false;
    };
    let Some(marker) = rest.chars().next() else {
        return false;
    };
    if !matches!(marker, '.' | ')') {
        return false;
    }

    rest[marker.len_utf8()..]
        .chars()
        .next()
        .is_none_or(char::is_whitespace)
}

fn colon_looks_structural(chars: &[char], index: usize) -> bool {
    let prev = chars[..index].iter().rev().find(|ch| !ch.is_whitespace());
    let next = chars[index + 1..].iter().find(|ch| !ch.is_whitespace());
    prev.zip(next).is_some_and(|(prev, next)| {
        is_json_like_wrapper_char(*prev) && is_json_like_wrapper_char(*next)
    })
}

fn is_json_like_wrapper_char(ch: char) -> bool {
    matches!(
        ch,
        '"' | '\'' | '[' | ']' | '{' | '}' | '(' | ')' | '-' | '0'..='9'
    )
}

fn extract_top_level_json_object_spans(raw: &str) -> Vec<(usize, usize)> {
    let mut objects = Vec::new();
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escape = false;

    for (index, ch) in raw.char_indices() {
        let Some(object_start) = start else {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        };

        if in_string {
            if escape {
                escape = false;
            } else if ch == '\\' {
                escape = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    objects.push((object_start, index + ch.len_utf8()));
                    start = None;
                }
            }
            _ => {}
        }
    }

    objects
}

/// Validate that the required string fields on a [`RawFact`] are
/// non-empty, returning `Some(message)` describing the first missing
/// field or `None` when the fact is well-formed.
pub fn validate_raw_fact(fact: &RawFact) -> Option<String> {
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
    fn parse_response_rejects_json_code_fence() {
        let error = parse_response("```json\n{\"facts\":[]}\n```").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn parse_response_accepts_leading_commentary_when_json_follows() {
        let parsed = parse_response("Sure:\n{\"facts\":[]}").unwrap();
        assert!(parsed.facts.is_empty());
    }

    #[test]
    fn parse_response_rejects_xml_tag_wrapper() {
        let error = parse_response("<response>{\"facts\":[]}</response>").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn parse_response_rejects_list_item_wrapper() {
        let error = parse_response("- {\"facts\":[]}").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn parse_response_rejects_multiple_json_objects() {
        let error = parse_response("{\"facts\":[]}{\"facts\":[]}").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn parse_response_rejects_schema_example_plus_answer() {
        let error = parse_response("The schema is {\"facts\":[]}\nActual answer: {\"facts\":[]}")
            .unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[test]
    fn parse_response_rejects_commentary_without_json_envelope() {
        let error = parse_response("Sure: I found a preference for coffee over tea.").unwrap_err();
        assert!(matches!(error, SlmError::Parse { .. }));
    }

    #[serial_test::serial]
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

    #[serial_test::serial]
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

    #[serial_test::serial]
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
