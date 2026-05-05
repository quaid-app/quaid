use quaid::core::conversation::extractor::compute_windows;
use quaid::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, ExtractionTriggerKind, Turn,
    TurnRole,
};

#[test]
fn compute_windows_should_slice_non_overlapping_batches_when_new_turns_fill_windows() {
    let conversation = conversation_with_cursor(0, 12);

    let windows = compute_windows(&conversation, ExtractionTriggerKind::Debounce, 5);

    assert_eq!(windows.len(), 3);
    assert_eq!(ordinals(&windows[0].new_turns), vec![1, 2, 3, 4, 5]);
    assert!(windows[0].lookback_turns.is_empty());
    assert_eq!(ordinals(&windows[1].new_turns), vec![6, 7, 8, 9, 10]);
    assert!(windows[1].lookback_turns.is_empty());
    assert_eq!(ordinals(&windows[2].new_turns), vec![11, 12]);
    assert!(windows[2].lookback_turns.is_empty());
}

#[test]
fn compute_windows_should_include_lookback_context_when_new_turns_are_sparse() {
    let conversation = conversation_with_cursor(10, 12);

    let windows = compute_windows(&conversation, ExtractionTriggerKind::Debounce, 5);

    assert_eq!(windows.len(), 1);
    assert_eq!(ordinals(&windows[0].new_turns), vec![11, 12]);
    assert_eq!(ordinals(&windows[0].lookback_turns), vec![8, 9, 10]);
    assert!(!windows[0].context_only);
}

#[test]
fn compute_windows_should_emit_context_only_flush_for_session_close_without_new_turns() {
    let conversation = conversation_with_cursor(12, 12);

    let windows = compute_windows(&conversation, ExtractionTriggerKind::SessionClose, 5);

    assert_eq!(windows.len(), 1);
    assert!(windows[0].new_turns.is_empty());
    assert_eq!(ordinals(&windows[0].lookback_turns), vec![8, 9, 10, 11, 12]);
    assert!(windows[0].context_only);
}

fn conversation_with_cursor(cursor: i64, last: i64) -> ConversationFile {
    ConversationFile {
        frontmatter: ConversationFrontmatter {
            file_type: "conversation".to_string(),
            session_id: "s1".to_string(),
            date: "2026-05-03".to_string(),
            started_at: "2026-05-03T10:00:00Z".to_string(),
            status: ConversationStatus::Open,
            closed_at: None,
            last_extracted_at: None,
            last_extracted_turn: cursor,
        },
        turns: (1..=last)
            .map(|ordinal| Turn {
                ordinal,
                role: if ordinal % 2 == 0 {
                    TurnRole::Assistant
                } else {
                    TurnRole::User
                },
                timestamp: format!("2026-05-03T10:00:{ordinal:02}Z"),
                content: format!("turn {ordinal}"),
                metadata: None,
            })
            .collect(),
    }
}

fn ordinals(turns: &[Turn]) -> Vec<i64> {
    turns.iter().map(|turn| turn.ordinal).collect()
}
