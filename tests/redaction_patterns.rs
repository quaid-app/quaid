#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions favour direct unwraps for readable failure output"
)]

//! Public-API unit tests for `quaid::core::redaction`: one test per secret
//! class plus blocklist, determinism, and the rehydrate round-trip. These
//! exercise the scrub engine in isolation; the MCP-surface integration
//! lives in `tests/mcp_redaction.rs`.

use quaid::core::redaction::{
    parse_blocklist, redact_mode_from_config, RedactMode, RedactionSession,
};

fn session() -> RedactionSession {
    RedactionSession::new(Vec::new())
}

#[test]
fn email_is_masked() {
    let mut s = session();
    let out = s.scrub("reach me at alice.smith+work@example.co.uk anytime");
    assert!(out.contains("<EMAIL_1>"), "got: {out}");
    assert!(!out.contains("alice.smith"), "email leaked: {out}");
}

#[test]
fn phone_is_masked() {
    let mut s = session();
    let out = s.scrub("call +1 (415) 555-0142 or 020 7946 0958");
    assert!(out.contains("<PHONE_1>"), "got: {out}");
    assert!(out.contains("<PHONE_2>"), "second phone not masked: {out}");
    assert!(!out.contains("555"), "phone digits leaked: {out}");
}

#[test]
fn openai_key_is_masked_as_secret() {
    let mut s = session();
    let out = s.scrub("key sk-proj-AbCdEf0123456789ZyXwVuTs in env");
    assert!(out.contains("<SECRET_1>"), "got: {out}");
    assert!(!out.contains("sk-proj-"), "key leaked: {out}");
}

#[test]
fn aws_access_key_is_masked_as_secret() {
    let mut s = session();
    let out = s.scrub("AKIAIOSFODNN7EXAMPLE is the access key id");
    assert!(out.contains("<SECRET_1>"), "got: {out}");
    assert!(!out.contains("AKIA"), "aws key leaked: {out}");
}

#[test]
fn github_token_is_masked_as_secret() {
    let mut s = session();
    let out = s.scrub("token ghp_1234567890abcdefghijABCDEFGHIJ0987 here");
    assert!(out.contains("<SECRET_1>"), "got: {out}");
    assert!(!out.contains("ghp_"), "gh token leaked: {out}");
}

#[test]
fn card_number_is_masked_as_account() {
    let mut s = session();
    let out = s.scrub("card 4111 1111 1111 1111 expires soon");
    assert!(out.contains("<ACCOUNT_1>"), "got: {out}");
    assert!(!out.contains("4111"), "card leaked: {out}");
}

#[test]
fn long_opaque_token_is_masked() {
    let mut s = session();
    // 40 hex chars — not an email/phone/card, caught by the token class.
    let out = s.scrub("digest a1b2c3d4e5f60718293a4b5c6d7e8f9012345678 committed");
    assert!(out.contains("<TOKEN_1>"), "got: {out}");
    assert!(!out.contains("a1b2c3d4"), "token leaked: {out}");
}

#[test]
fn blocklist_literal_is_masked() {
    let mut s = RedactionSession::new(vec!["ProjectBluebird".to_string()]);
    let out = s.scrub("we shipped ProjectBluebird and projectbluebird again");
    assert!(
        !out.to_lowercase().contains("projectbluebird"),
        "blocklist leaked: {out}"
    );
    // Same token reused for both case variants (stable per original casing
    // is not required; both distinct casings each get a stable token).
    assert!(out.contains("<SECRET_"), "blocklist not tokenised: {out}");
}

#[test]
fn hyphenated_uuid_is_not_masked() {
    let mut s = session();
    // Page `quaid_id` shape — a structural identifier, not a secret. The
    // hyphens split it below the 32-char token threshold.
    let uuid = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    let out = s.scrub(&format!("quaid_id: {uuid} stays"));
    assert!(out.contains(uuid), "uuid was wrongly masked: {out}");
    assert_eq!(s.masked_count(), 0);
}

#[test]
fn deterministic_tokens_within_session() {
    let mut s = session();
    let out = s.scrub("a@x.com then a@x.com then b@x.com");
    // Same email -> same token; different email -> different token.
    assert_eq!(out.matches("<EMAIL_1>").count(), 2, "got: {out}");
    assert!(
        out.contains("<EMAIL_2>"),
        "second distinct email missing: {out}"
    );
    assert_eq!(s.masked_count(), 2);
}

#[test]
fn no_match_returns_input_unchanged() {
    let mut s = session();
    let input = "just some ordinary prose with no secrets in it at all.";
    assert_eq!(s.scrub(input), input);
    assert_eq!(s.masked_count(), 0);
}

#[test]
fn rehydrate_round_trips_scrubbed_text() {
    let mut s = session();
    let original = "email alice@example.com and key sk-AbCdEf0123456789ZyXwVuTs done";
    let scrubbed = s.scrub(original);
    assert_ne!(scrubbed, original);
    let restored = s.rehydrate(&scrubbed);
    assert_eq!(restored, original, "round-trip mismatch");
}

#[test]
fn rehydrate_leaves_unknown_tokens_untouched() {
    let s = session();
    // No mapping recorded -> nothing to reverse.
    let text = "this mentions <EMAIL_9> which was never assigned";
    assert_eq!(s.rehydrate(text), text);
}

#[test]
fn redact_mode_parsing() {
    assert_eq!(
        RedactMode::from_config_value("patterns"),
        RedactMode::Patterns
    );
    assert_eq!(
        RedactMode::from_config_value("PATTERNS"),
        RedactMode::Patterns
    );
    assert_eq!(RedactMode::from_config_value("off"), RedactMode::Off);
    assert_eq!(RedactMode::from_config_value("garbage"), RedactMode::Off);
    assert_eq!(redact_mode_from_config(None), RedactMode::Off);
    assert_eq!(
        redact_mode_from_config(Some("patterns")),
        RedactMode::Patterns
    );
    assert!(RedactMode::Patterns.is_active());
    assert!(!RedactMode::Off.is_active());
}

#[test]
fn blocklist_parsing_splits_and_trims() {
    let parsed = parse_blocklist(Some("alpha, beta\n  gamma  ,,"));
    assert_eq!(parsed, vec!["alpha", "beta", "gamma"]);
    assert!(parse_blocklist(None).is_empty());
    assert!(parse_blocklist(Some("   ")).is_empty());
}
