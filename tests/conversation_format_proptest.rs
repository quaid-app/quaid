#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Property test for the documented conversation-format round-trip invariant
//! `render(parse(x)) == x` (see `src/core/conversation/format.rs`).
//!
//! The proptest drives the canonical direction: build an arbitrary
//! [`ConversationFile`], render it, parse the rendered text back, and assert
//! `render(parse_str(render(file))) == render(file)`. Rendering is the
//! canonical normal form, so this is the strongest stable statement of the
//! round-trip that does not depend on whatever non-canonical whitespace a
//! generator happens to emit.
//!
//! GENERATOR CAP / TODO(#226): on this branch the format does **not** escape
//! turn content that contains the structural markers — the turn-boundary
//! comment (`<!-- quaid-turn-boundary -->`), a `## Turn ` heading, a leading
//! legacy `---` boundary, or a ```` ```json turn-metadata ```` fence. Feeding
//! such content through `render` produces bytes that `parse_str` re-segments
//! into a *different* turn structure, so the round-trip genuinely fails for
//! that adversarial input. Format-v2 escaping (PR #226, not on this branch)
//! is the fix; until it lands, the content generator below deliberately
//! excludes those marker shapes rather than reimplementing escaping here. The
//! `round_trip_breaks_on_unescaped_turn_boundary` test pins the *known* break
//! so #226 has a regression target to flip green.

use proptest::prelude::*;

use quaid::core::conversation::format::{parse_str, render};
use quaid::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, Turn, TurnRole,
};

const TURN_BOUNDARY: &str = "<!-- quaid-turn-boundary -->";
const METADATA_FENCE_OPEN: &str = "```json turn-metadata";

/// A single content line that `render`/`parse_str` round-trips losslessly:
/// no trailing whitespace, no carriage return, and not a structural marker
/// the unescaped format would mis-segment.
fn safe_content_line() -> impl Strategy<Value = String> {
    // A small alphabet of letters, digits, punctuation, whitespace, and a few
    // multibyte code points to exercise the Unicode-char path without tripping
    // the structural markers.
    "[a-zA-Z0-9 .,?!:;@#%&*()/'\u{00e9}\u{4e2d}\u{1f600}-]{0,40}".prop_filter(
        "no marker-shaped or trailing-whitespace lines",
        |line: &String| {
            let trimmed = line.trim_end();
            trimmed == line.as_str()
                && !line.starts_with("## Turn ")
                && !line.starts_with("---")
                && !line.starts_with("```")
                && !line.starts_with("~~~")
                && line.trim() != METADATA_FENCE_OPEN
                && line.trim() != "```"
                && !line.contains(TURN_BOUNDARY)
                && !line.contains('\u{2028}')
                && !line.contains('\u{2029}')
        },
    )
}

/// Multi-line content with safe interior lines. The first and last lines are
/// non-blank so `render`'s content-boundary trimming is a no-op (blank
/// boundary lines are collapsed by `parse_str`, which is a separate,
/// documented normalization rather than a round-trip property).
fn safe_content() -> impl Strategy<Value = String> {
    proptest::collection::vec(safe_content_line(), 1..6).prop_map(|mut lines| {
        if lines.first().map(|l| l.trim().is_empty()).unwrap_or(true) {
            lines[0] = "x".to_owned();
        }
        let last = lines.len() - 1;
        if lines[last].trim().is_empty() {
            lines[last] = "y".to_owned();
        }
        lines.join("\n")
    })
}

/// RFC-3339-ish timestamp; only the textual shape matters for the round-trip,
/// and it must not contain the ` · ` heading separator or a newline.
fn safe_timestamp() -> impl Strategy<Value = String> {
    (
        2020i32..2030,
        1u32..13,
        1u32..28,
        0u32..24,
        0u32..60,
        0u32..60,
    )
        .prop_map(|(y, mo, d, h, mi, s)| format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z"))
}

fn turn_role() -> impl Strategy<Value = TurnRole> {
    prop_oneof![
        Just(TurnRole::User),
        Just(TurnRole::Assistant),
        Just(TurnRole::System),
        Just(TurnRole::Tool),
    ]
}

/// A turn with no metadata (the ordinal is assigned by the file strategy so
/// the sequence is 1..=N, which `parse_str` re-derives from the headings).
fn turn_body() -> impl Strategy<Value = (TurnRole, String, String, Option<serde_json::Value>)> {
    let metadata = prop_oneof![
        Just(None),
        any::<u8>().prop_map(|n| Some(serde_json::json!({ "importance": n }))),
    ];
    (turn_role(), safe_timestamp(), safe_content(), metadata)
}

fn conversation_file() -> impl Strategy<Value = ConversationFile> {
    let frontmatter = (
        "[a-z0-9-]{1,20}",
        safe_timestamp(),
        prop_oneof![
            Just(ConversationStatus::Open),
            Just(ConversationStatus::Closed)
        ],
        0i64..50,
    )
        .prop_map(|(session_id, started_at, status, last_turn)| {
            let date = started_at.get(..10).unwrap_or("2026-01-01").to_owned();
            let closed_at = if matches!(status, ConversationStatus::Closed) {
                Some(started_at.clone())
            } else {
                None
            };
            ConversationFrontmatter {
                file_type: "conversation".to_owned(),
                session_id,
                date,
                started_at,
                status,
                closed_at,
                last_extracted_at: None,
                last_extracted_turn: last_turn,
                format_version: quaid::core::types::CONVERSATION_FORMAT_VERSION,
            }
        });

    (frontmatter, proptest::collection::vec(turn_body(), 0..6)).prop_map(|(frontmatter, bodies)| {
        let turns = bodies
            .into_iter()
            .enumerate()
            .map(|(index, (role, timestamp, content, metadata))| Turn {
                ordinal: i64::try_from(index + 1).expect("ordinal fits i64"),
                role,
                timestamp,
                content,
                metadata,
            })
            .collect();
        ConversationFile { frontmatter, turns }
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// The canonical round-trip: rendered bytes parse back to a file that
    /// renders identically. `render` is the normal form, so a stable fixed
    /// point here is exactly the `render(parse(x)) == x` invariant for any
    /// `x` that is itself canonical output.
    #[test]
    fn render_parse_render_is_a_fixed_point(file in conversation_file()) {
        let rendered = render(&file);
        let parsed = parse_str(&rendered)
            .unwrap_or_else(|err| panic!("parse_str rejected canonical render: {err}\n---\n{rendered}"));
        prop_assert_eq!(render(&parsed), rendered);
    }

    /// Stronger: every structural field survives the round-trip, not just the
    /// rendered bytes.
    #[test]
    fn round_trip_preserves_turn_structure(file in conversation_file()) {
        let parsed = parse_str(&render(&file)).expect("parse canonical render");
        prop_assert_eq!(parsed.turns.len(), file.turns.len());
        for (original, round_tripped) in file.turns.iter().zip(parsed.turns.iter()) {
            prop_assert_eq!(original.ordinal, round_tripped.ordinal);
            prop_assert_eq!(&original.role, &round_tripped.role);
            prop_assert_eq!(&original.timestamp, &round_tripped.timestamp);
            prop_assert_eq!(&original.content, &round_tripped.content);
            prop_assert_eq!(&original.metadata, &round_tripped.metadata);
        }
    }
}

/// Pins the KNOWN round-trip break that the generator caps out: a turn whose
/// content embeds the literal turn-boundary marker is re-segmented into an
/// extra turn on parse. This is the exact case format-v2 escaping (#226) must
/// fix; when it lands, this test should be inverted to assert the round-trip
/// holds. Keeping it as an explicit `!=` assertion documents the limitation
/// instead of silently excluding it.
#[test]
fn round_trip_breaks_on_unescaped_turn_boundary() {
    let file = ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_owned(),
            session_id: "s1".to_owned(),
            date: "2026-01-01".to_owned(),
            started_at: "2026-01-01T00:00:00Z".to_owned(),
            status: ConversationStatus::Open,
            closed_at: None,
            last_extracted_at: None,
            last_extracted_turn: 0,
            format_version: quaid::core::types::CONVERSATION_FORMAT_VERSION,
        },
        turns: vec![Turn {
            ordinal: 1,
            role: TurnRole::User,
            timestamp: "2026-01-01T00:00:00Z".to_owned(),
            // Embedding the boundary marker mid-content is the adversarial case.
            content: format!("before\n{TURN_BOUNDARY}\nafter"),
            metadata: None,
        }],
    };

    let rendered = render(&file);
    let original_content = file.turns[0].content.clone();
    // On this branch the embedded boundary is NOT escaped, so the round-trip
    // is lossy: parse either errors (the post-boundary text has no `## Turn`
    // heading) or yields a turn whose content lost the boundary segment.
    let faithfully_round_tripped = matches!(
        parse_str(&rendered),
        Ok(ref parsed)
            if parsed.turns.len() == 1 && parsed.turns[0].content == original_content
    );
    // TODO(#226): when format-v2 escaping lands, invert this to
    // `assert!(faithfully_round_tripped)` and drop the generator cap above.
    assert!(
        !faithfully_round_tripped,
        "unexpected: embedded turn-boundary round-tripped faithfully — \
         format-v2 escaping (#226) may have landed; invert this test and lift \
         the generator cap in this file"
    );
}
