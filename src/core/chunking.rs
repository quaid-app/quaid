use sha2::{Digest, Sha256};

use super::types::{Chunk, Page};

/// Split a page into truth-section and timeline-entry chunks.
pub fn chunk_page(page: &Page) -> Vec<Chunk> {
    let mut chunks = truth_chunks(page);
    chunks.extend(timeline_chunks(page));
    chunks
}

fn truth_chunks(page: &Page) -> Vec<Chunk> {
    let content = page.compiled_truth.trim();
    if content.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current_heading = String::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut saw_heading = false;

    for line in page.compiled_truth.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            saw_heading = true;
            if !current_lines.is_empty() {
                chunks.push(build_chunk(
                    &page.slug,
                    &current_heading,
                    &current_lines.join("\n"),
                    "truth_section",
                ));
                current_lines.clear();
            }
            current_heading = heading.trim().to_owned();
        }

        current_lines.push(line.to_owned());
    }

    if !current_lines.is_empty() {
        let heading = if saw_heading {
            current_heading.as_str()
        } else {
            ""
        };
        chunks.push(build_chunk(
            &page.slug,
            heading,
            &current_lines.join("\n"),
            "truth_section",
        ));
    }

    chunks
}

fn timeline_chunks(page: &Page) -> Vec<Chunk> {
    let timeline = page.timeline.trim();
    if timeline.is_empty() {
        return Vec::new();
    }

    split_timeline_entries(&page.timeline)
        .into_iter()
        .map(|entry| {
            let heading_path = entry
                .lines()
                .find_map(|line| {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                })
                .unwrap_or_default();

            build_chunk(&page.slug, &heading_path, &entry, "timeline_entry")
        })
        .collect()
}

fn split_timeline_entries(timeline: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = Vec::new();

    for line in timeline.lines() {
        if is_boundary(line) {
            if !current.is_empty() {
                entries.push(current.join("\n"));
                current.clear();
            }
            continue;
        }

        current.push(line.to_owned());
    }

    if !current.is_empty() {
        entries.push(current.join("\n"));
    }

    entries
        .into_iter()
        .filter(|entry| !entry.trim().is_empty())
        .collect()
}

fn build_chunk(page_slug: &str, heading_path: &str, content: &str, chunk_type: &str) -> Chunk {
    Chunk {
        page_slug: page_slug.to_owned(),
        heading_path: heading_path.to_owned(),
        content: content.to_owned(),
        content_hash: content_hash(content),
        token_count: std::cmp::max(1, content.len() / 4),
        chunk_type: chunk_type.to_owned(),
    }
}

fn is_boundary(line: &str) -> bool {
    line.trim_end_matches('\r') == "---"
}

fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    let mut hash = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hash.push_str(&format!("{byte:02x}"));
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_page(compiled_truth: &str, timeline: &str) -> Page {
        Page {
            slug: "people/alice".to_owned(),
            uuid: "01969f11-9448-7d79-8d3f-c68f54761234".to_owned(),
            page_type: "person".to_owned(),
            superseded_by: None,
            title: "Alice".to_owned(),
            summary: "Founder".to_owned(),
            compiled_truth: compiled_truth.to_owned(),
            timeline: timeline.to_owned(),
            frontmatter: crate::core::types::Frontmatter::new(),
            wing: "people".to_owned(),
            room: String::new(),
            version: 1,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
            updated_at: "2024-01-01T00:00:00Z".to_owned(),
            truth_updated_at: "2024-01-01T00:00:00Z".to_owned(),
            timeline_updated_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    #[test]
    fn chunk_page_creates_three_truth_chunks_from_three_sections() {
        let page = test_page(
            "## State\nAlice is investing.\n## Assessment\nStrong operator.\n## Network\nKnows top founders.",
            "",
        );

        let chunks = chunk_page(&page);
        let truth_chunks: Vec<_> = chunks
            .iter()
            .filter(|chunk| chunk.chunk_type == "truth_section")
            .collect();

        assert_eq!(truth_chunks.len(), 3);
        assert_eq!(truth_chunks[0].heading_path, "State");
        assert_eq!(truth_chunks[1].heading_path, "Assessment");
        assert_eq!(truth_chunks[2].heading_path, "Network");
    }

    #[test]
    fn chunk_page_creates_five_timeline_chunks_from_separated_entries() {
        let page = test_page(
            "",
            "2024-01-01 Joined Acme\n---\n2024-02-01 Raised seed\n---\n2024-03-01 Hired CTO\n---\n2024-04-01 Shipped beta\n---\n2024-05-01 Closed Series A",
        );

        let chunks = chunk_page(&page);
        let timeline_chunks: Vec<_> = chunks
            .iter()
            .filter(|chunk| chunk.chunk_type == "timeline_entry")
            .collect();

        assert_eq!(timeline_chunks.len(), 5);
    }

    #[test]
    fn chunk_page_sets_non_empty_content_hash_for_every_chunk() {
        let page = test_page(
            "## State\nAlice is investing.\n## Assessment\nStrong operator.",
            "2024-01-01 Joined Acme\n---\n2024-02-01 Raised seed",
        );

        let chunks = chunk_page(&page);

        assert!(chunks.iter().all(|chunk| !chunk.content_hash.is_empty()));
    }
}
