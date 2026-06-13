//! Outbound secret scrubbing for the MCP read surface.
//!
//! This module implements phase 1 of issue #159: a deterministic,
//! pattern-based scrub that runs at the MCP serialization choke point
//! (`crate::mcp::tools::{pages,search,conversation}`) just before page
//! content is handed to a (frequently cloud-hosted) LLM client. It is
//! deliberately *outbound only*: FTS5 and embeddings always index the
//! original text, so retrieval quality is unaffected and the scrub only
//! changes what crosses the wire.
//!
//! What this is NOT: this is **secret scrubbing**, not full PII / NER
//! redaction. It catches high-confidence machine-shaped secrets (emails,
//! phone numbers, API-key shapes, account / card numbers) plus a
//! user-supplied blocklist. Names and other free-text PII are explicitly
//! deferred to a later phase (#159 notes `crate::core::entities` patterns
//! can feed a future NER pass).
//!
//! Determinism: within a single [`crate::core::redaction::RedactionSession`]
//! the same original value always maps to the same token (`<EMAIL_1>`,
//! `<SECRET_2>`, ...), preserving coreference so an LLM can still reason
//! about "the same key" appearing twice. The session also retains the
//! reverse mapping so the `memory_rehydrate` MCP tool can turn tokens back
//! into the originals locally, without ever sending them over the wire.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

/// Outbound redaction mode, parsed from the `mcp.redact_outbound` config key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactMode {
    /// No scrubbing — output is byte-identical to the unredacted payload.
    Off,
    /// Deterministic pattern + blocklist scrubbing (phase 1).
    Patterns,
}

impl RedactMode {
    /// Parse the `mcp.redact_outbound` config value. Unknown values fall
    /// back to [`RedactMode::Off`] so a malformed config never silently
    /// turns scrubbing on (or, worse, errors out a read tool).
    pub fn from_config_value(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "patterns" => RedactMode::Patterns,
            _ => RedactMode::Off,
        }
    }

    /// Whether this mode performs any scrubbing.
    pub fn is_active(self) -> bool {
        matches!(self, RedactMode::Patterns)
    }
}

/// A single secret class. The ordering of [`PATTERN_CLASSES`] is the match
/// priority: more specific shapes (keys, cards) are scrubbed before the
/// broad token shape so an API key is never mislabelled `<TOKEN_1>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretClass {
    Email,
    Phone,
    ApiKey,
    AccountNumber,
    Token,
}

impl SecretClass {
    /// Token prefix used in the placeholder, e.g. `EMAIL` -> `<EMAIL_1>`.
    fn token_prefix(self) -> &'static str {
        match self {
            SecretClass::Email => "EMAIL",
            SecretClass::Phone => "PHONE",
            SecretClass::ApiKey => "SECRET",
            SecretClass::AccountNumber => "ACCOUNT",
            SecretClass::Token => "TOKEN",
        }
    }
}

struct PatternClass {
    class: SecretClass,
    regex: Regex,
}

/// Compiled secret-shape regexes, ordered by match priority. Built once per
/// process via [`LazyLock`]. Patterns are intentionally conservative — the
/// feature is opt-in, so false positives that mangle context are the bigger
/// risk than a missed secret (#159 risk note).
static PATTERN_CLASSES: LazyLock<Vec<PatternClass>> = LazyLock::new(|| {
    // reason: these literals are authored and tested constants; a panic here
    // would fire at first use in every build and is caught by unit tests.
    #[allow(clippy::expect_used)]
    fn compile(pattern: &str) -> Regex {
        Regex::new(pattern).expect("redaction pattern must compile")
    }

    vec![
        // Emails: local-part@domain.tld. Kept first so an address is never
        // shredded into account/token fragments.
        PatternClass {
            class: SecretClass::Email,
            regex: compile(r"(?i)\b[a-z0-9._%+\-]+@[a-z0-9.\-]+\.[a-z]{2,}\b"),
        },
        // Provider API-key shapes: OpenAI-style `sk-...`/`sk-proj-...`,
        // AWS access keys `AKIA...`, GitHub tokens `ghp_`/`gho_`/`ghs_`.
        PatternClass {
            class: SecretClass::ApiKey,
            regex: compile(
                r"(?x)
                \b(?:
                    sk-(?:proj-)?[A-Za-z0-9_\-]{16,}
                  | (?:AKIA|ASIA)[A-Z0-9]{16}
                  | gh[posru]_[A-Za-z0-9]{20,}
                  | xox[baprs]-[A-Za-z0-9\-]{10,}
                )\b",
            ),
        },
        // Payment-card / account numbers: 13-19 digits, optionally grouped
        // by spaces or dashes in 4s. Checked before the bare token shape so
        // a card number is labelled `<ACCOUNT_n>` not `<TOKEN_n>`.
        PatternClass {
            class: SecretClass::AccountNumber,
            regex: compile(r"\b(?:\d[ \-]?){12,18}\d\b"),
        },
        // Phone numbers: optional +country code, separators, grouped digits.
        // The `regex` crate has no look-around, so we anchor on a word
        // boundary plus an optional leading `+` and require at least two
        // separated digit groups to avoid swallowing bare integers.
        PatternClass {
            class: SecretClass::Phone,
            regex: compile(
                r"(?x)
                \+?\d{1,3}[\s.\-](?:\(\d{1,4}\)[\s.\-]?)?\d{2,4}(?:[\s.\-]\d{2,4}){1,3}
                \b",
            ),
        },
        // Long opaque tokens: 32+ chars of base64/hex-ish alphabet. Catches
        // bearer tokens and hex secrets the named shapes above miss.
        //
        // The alphabet deliberately excludes `-` so hyphenated UUIDs
        // (`xxxxxxxx-xxxx-...`, a common structural identifier such as a
        // page `quaid_id`) split at their hyphens into sub-32 segments and
        // are NOT masked. `sk-…`/`AKIA…`-style hyphenated keys are already
        // caught by the higher-priority API-key class above.
        PatternClass {
            class: SecretClass::Token,
            regex: compile(r"\b[A-Za-z0-9+/_]{32,}={0,2}\b"),
        },
    ]
});

/// Per-connection redaction state: deterministic forward (original -> token)
/// and reverse (token -> original) maps plus per-class counters.
///
/// One of these lives behind a `Mutex` on `crate::mcp::server::QuaidServer`,
/// which is per-connection (each stdio session / SSE connection gets a fresh
/// `QuaidServer`), so token numbering is stable for the life of a connection
/// and never leaks across clients.
#[derive(Debug, Default)]
pub struct RedactionSession {
    /// original secret -> assigned token
    forward: HashMap<String, String>,
    /// token -> original secret
    reverse: HashMap<String, String>,
    /// next index per token prefix (e.g. "EMAIL" -> 3)
    counters: HashMap<&'static str, usize>,
    /// user-defined literal blocklist (longest-first for greedy matching)
    blocklist: Vec<String>,
}

impl RedactionSession {
    /// Build a session with an optional user-defined blocklist. Blocklist
    /// entries are matched as plain (case-insensitive) substrings and are
    /// scrubbed before the regex classes, so a custom secret always wins
    /// over a generic shape.
    pub fn new(blocklist: Vec<String>) -> Self {
        let mut blocklist: Vec<String> = blocklist
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        // Longest-first so overlapping entries scrub the larger secret.
        blocklist.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        blocklist.dedup();
        Self {
            blocklist,
            ..Self::default()
        }
    }

    /// Number of distinct secrets masked so far in this session.
    pub fn masked_count(&self) -> usize {
        self.forward.len()
    }

    /// Assign (or reuse) a deterministic token for one original value under
    /// the given class prefix.
    fn token_for(&mut self, prefix: &'static str, original: &str) -> String {
        if let Some(existing) = self.forward.get(original) {
            return existing.clone();
        }
        let counter = self.counters.entry(prefix).or_insert(0);
        *counter += 1;
        let token = format!("<{prefix}_{counter}>");
        self.forward.insert(original.to_string(), token.clone());
        self.reverse.insert(token.clone(), original.to_string());
        token
    }

    /// Scrub `text`, replacing every recognised secret with a deterministic
    /// token and recording the mapping. Returns the scrubbed string. When
    /// nothing matches, the returned string equals the input.
    pub fn scrub(&mut self, text: &str) -> String {
        // Blocklist first (case-insensitive literal substrings), then the
        // ordered regex classes. Each pass operates on the output of the
        // previous so a token emitted by one pass is never re-scrubbed by a
        // later one (tokens contain `<`/`>`, outside every pattern alphabet).
        let mut out = self.scrub_blocklist(text);
        for pattern in PATTERN_CLASSES.iter() {
            out = self.scrub_with(&pattern.regex, pattern.class.token_prefix(), &out);
        }
        out
    }

    fn scrub_blocklist(&mut self, text: &str) -> String {
        if self.blocklist.is_empty() {
            return text.to_string();
        }
        let mut out = text.to_string();
        // Clone the entries to avoid borrowing `self` while we mutate the maps.
        let entries = self.blocklist.clone();
        for entry in entries {
            // Case-insensitive find loop preserving the matched original case.
            let mut search_from = 0usize;
            let lower_entry = entry.to_ascii_lowercase();
            loop {
                let haystack = out[search_from..].to_ascii_lowercase();
                let Some(rel) = haystack.find(&lower_entry) else {
                    break;
                };
                let start = search_from + rel;
                let end = start + entry.len();
                let matched = out[start..end].to_string();
                let token = self.token_for(SecretClass::ApiKey.token_prefix(), &matched);
                out.replace_range(start..end, &token);
                search_from = start + token.len();
            }
        }
        out
    }

    fn scrub_with(&mut self, regex: &Regex, prefix: &'static str, text: &str) -> String {
        // Collect match spans first (immutable borrow of `text`).
        let spans: Vec<(usize, usize, String)> = regex
            .find_iter(text)
            .map(|m| (m.start(), m.end(), m.as_str().to_string()))
            .collect();
        if spans.is_empty() {
            return text.to_string();
        }
        // Assign tokens in document order so numbering reads left-to-right,
        // then rewrite right-to-left so byte offsets stay valid as we splice.
        let tokens: Vec<(usize, usize, String)> = spans
            .into_iter()
            .map(|(start, end, matched)| (start, end, self.token_for(prefix, &matched)))
            .collect();
        let mut out = text.to_string();
        for (start, end, token) in tokens.into_iter().rev() {
            out.replace_range(start..end, &token);
        }
        out
    }

    /// Reverse the session mapping: replace every known token in `text` with
    /// its original value. Unknown tokens are left untouched. This is the
    /// inverse of [`RedactionSession::scrub`] for the same session.
    pub fn rehydrate(&self, text: &str) -> String {
        if self.reverse.is_empty() {
            return text.to_string();
        }
        // Token shape is `<PREFIX_n>`; match it generically and look each up.
        static TOKEN_RE: LazyLock<Regex> = LazyLock::new(|| {
            #[allow(clippy::expect_used)]
            Regex::new(r"<[A-Z]+_\d+>").expect("token regex must compile")
        });
        let spans: Vec<(usize, usize, String)> = TOKEN_RE
            .find_iter(text)
            .filter_map(|m| {
                self.reverse
                    .get(m.as_str())
                    .map(|orig| (m.start(), m.end(), orig.clone()))
            })
            .collect();
        if spans.is_empty() {
            return text.to_string();
        }
        let mut out = text.to_string();
        for (start, end, original) in spans.into_iter().rev() {
            out.replace_range(start..end, &original);
        }
        out
    }
}

/// Read the configured outbound-redaction mode from the `config` table.
pub fn redact_mode_from_config(value: Option<&str>) -> RedactMode {
    value
        .map(RedactMode::from_config_value)
        .unwrap_or(RedactMode::Off)
}

/// Parse the user-defined blocklist config value (`mcp.redact_blocklist`).
///
/// The value is a comma- or newline-separated list of literal secrets to
/// always scrub (e.g. an internal project codename). Empty / whitespace
/// entries are dropped.
pub fn parse_blocklist(value: Option<&str>) -> Vec<String> {
    value
        .map(|raw| {
            raw.split(['\n', ','])
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}
