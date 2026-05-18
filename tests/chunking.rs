use quaid::core::chunking::chunk_page;
use quaid::core::types::{Frontmatter, Page};

fn test_page(compiled_truth: &str, timeline: &str) -> Page {
    Page {
        slug: "conversations/2026-05-18/playground".to_owned(),
        uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_owned(),
        page_type: "conversation".to_owned(),
        superseded_by: None,
        title: "Playground".to_owned(),
        summary: "Conversation".to_owned(),
        compiled_truth: compiled_truth.to_owned(),
        timeline: timeline.to_owned(),
        frontmatter: Frontmatter::new(),
        wing: "conversations".to_owned(),
        room: String::new(),
        version: 1,
        created_at: "2026-05-18T00:00:00Z".to_owned(),
        updated_at: "2026-05-18T00:00:00Z".to_owned(),
        truth_updated_at: "2026-05-18T00:00:00Z".to_owned(),
        timeline_updated_at: "2026-05-18T00:00:00Z".to_owned(),
    }
}

#[test]
fn chunk_page_skips_leading_blank_truth_before_first_heading() {
    let page = test_page(
        "\n## Turn 1 · user · 2026-05-18T03:36:24Z\n\nI like coffee more than tea.",
        "",
    );

    let chunks = chunk_page(&page);

    assert_eq!(chunks.len(), 1);
    assert_eq!(
        chunks[0].heading_path,
        "Turn 1 · user · 2026-05-18T03:36:24Z"
    );
    assert!(chunks[0].content.contains("I like coffee more than tea."));
    assert!(chunks.iter().all(|chunk| !chunk.content.trim().is_empty()));
}
