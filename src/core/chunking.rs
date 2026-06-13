use sha2::{Digest, Sha256};

use super::types::{Chunk, Page};

/// Approximate bytes per token used by the `len / 4` token estimate.
const APPROX_BYTES_PER_TOKEN: usize = 4;

/// Chunks whose estimated token count exceeds this cap are sub-split so the
/// whole chunk fits the encoder's 512-token window, with margin for special
/// tokens and tokenizer variance. Without the cap, a heading-less page becomes
/// one whole-page chunk silently truncated at 512 tokens — everything past
/// roughly the first ~2,000 characters never reaches the vector index.
const MAX_CHUNK_TOKENS: usize = 480;

/// Target window size (estimated tokens) for sub-split chunk parts.
const SPLIT_WINDOW_TOKENS: usize = 440;

/// Approximate context overlap (estimated tokens) carried between consecutive
/// sub-split parts so facts near a window boundary stay intact in at least
/// one window.
const SPLIT_OVERLAP_TOKENS: usize = 60;

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

    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            saw_heading = true;
            if !current_lines.is_empty() {
                push_capped_chunks(
                    &mut chunks,
                    &page.slug,
                    &current_heading,
                    &current_lines.join("\n"),
                    "truth_section",
                );
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
        push_capped_chunks(
            &mut chunks,
            &page.slug,
            heading,
            &current_lines.join("\n"),
            "truth_section",
        );
    }

    chunks
}

fn timeline_chunks(page: &Page) -> Vec<Chunk> {
    let timeline = page.timeline.trim();
    if timeline.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    for entry in split_timeline_entries(&page.timeline) {
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

        push_capped_chunks(
            &mut chunks,
            &page.slug,
            &heading_path,
            &entry,
            "timeline_entry",
        );
    }
    chunks
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

/// Push `content` as one chunk when it fits [`MAX_CHUNK_TOKENS`], otherwise
/// sub-split it into overlapping windows of roughly [`SPLIT_WINDOW_TOKENS`],
/// suffixing each part's heading path with its part index (e.g. `" [2/3]"`).
fn push_capped_chunks(
    chunks: &mut Vec<Chunk>,
    page_slug: &str,
    heading_path: &str,
    content: &str,
    chunk_type: &str,
) {
    let windows = split_oversized(content);
    if windows.len() <= 1 {
        chunks.push(build_chunk(page_slug, heading_path, content, chunk_type));
        return;
    }

    let total = windows.len();
    for (index, window) in windows.iter().enumerate() {
        let part_heading = part_heading_path(heading_path, index + 1, total);
        chunks.push(build_chunk(page_slug, &part_heading, window, chunk_type));
    }
}

fn part_heading_path(heading_path: &str, part: usize, total: usize) -> String {
    if heading_path.is_empty() {
        format!("[{part}/{total}]")
    } else {
        format!("{heading_path} [{part}/{total}]")
    }
}

/// Split `content` into overlapping windows whose `len / 4` token estimate
/// stays under [`MAX_CHUNK_TOKENS`], breaking at paragraph boundaries first,
/// then line boundaries, then (as a last resort) character boundaries.
/// Content already under the cap is returned as a single window.
fn split_oversized(content: &str) -> Vec<&str> {
    if content.len() <= MAX_CHUNK_TOKENS * APPROX_BYTES_PER_TOKEN {
        return vec![content];
    }

    let window_bytes = SPLIT_WINDOW_TOKENS * APPROX_BYTES_PER_TOKEN;
    let overlap_bytes = SPLIT_OVERLAP_TOKENS * APPROX_BYTES_PER_TOKEN;
    let atoms = split_atoms(content, window_bytes);
    if atoms.is_empty() {
        return vec![content];
    }

    let mut windows = Vec::new();
    let mut window_start_atom = 0;
    loop {
        let window_byte_start = atoms[window_start_atom].0;
        let mut end_atom = window_start_atom;
        while end_atom + 1 < atoms.len()
            && atoms[end_atom + 1].1 - window_byte_start <= window_bytes
        {
            end_atom += 1;
        }
        let window_byte_end = atoms[end_atom].1;
        windows.push(&content[window_byte_start..window_byte_end]);
        if end_atom + 1 >= atoms.len() {
            return windows;
        }

        // Start the next window inside this one's tail so consecutive parts
        // share roughly SPLIT_OVERLAP_TOKENS of context: walk back over atoms
        // that begin inside the overlap region, falling back to carrying the
        // final atom when it is small enough.
        let desired_overlap_start = window_byte_end.saturating_sub(overlap_bytes);
        let mut next_start = end_atom + 1;
        while next_start > window_start_atom + 1 && atoms[next_start - 1].0 >= desired_overlap_start
        {
            next_start -= 1;
        }
        if next_start > end_atom
            && end_atom > window_start_atom
            && window_byte_end - atoms[end_atom].0 <= 2 * overlap_bytes
        {
            next_start = end_atom;
        }
        window_start_atom = next_start;
    }
}

/// Split `content` into byte ranges of at most `max_atom_bytes`, preferring
/// whole paragraphs, then single lines, then hard character splits. Blank
/// lines and trailing whitespace are excluded from the ranges.
fn split_atoms(content: &str, max_atom_bytes: usize) -> Vec<(usize, usize)> {
    let mut atoms = Vec::new();
    for (start, end) in paragraph_ranges(content) {
        if end - start <= max_atom_bytes {
            atoms.push((start, end));
            continue;
        }
        for (line_start, line_end) in line_ranges(&content[start..end], start) {
            if line_end - line_start <= max_atom_bytes {
                atoms.push((line_start, line_end));
            } else {
                hard_split(content, line_start, line_end, max_atom_bytes, &mut atoms);
            }
        }
    }
    atoms
}

/// Byte ranges of paragraphs (runs of non-blank lines), with trailing
/// whitespace trimmed from each range.
fn paragraph_ranges(content: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut start: Option<usize> = None;
    let mut end = 0;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        if line.trim().is_empty() {
            if let Some(paragraph_start) = start.take() {
                ranges.push((paragraph_start, end));
            }
        } else {
            if start.is_none() {
                start = Some(line_start);
            }
            end = line_start + line.trim_end().len();
        }
    }
    if let Some(paragraph_start) = start {
        ranges.push((paragraph_start, end));
    }
    ranges
}

/// Byte ranges (relative to the whole content via `base`) of the non-blank
/// lines of `paragraph`, with trailing whitespace trimmed.
fn line_ranges(paragraph: &str, base: usize) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for line in paragraph.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            ranges.push((base + line_start, base + line_start + trimmed.len()));
        }
    }
    ranges
}

/// Last-resort split of an unbreakable span into ranges of at most
/// `max_atom_bytes`, cutting only at UTF-8 character boundaries.
fn hard_split(
    content: &str,
    start: usize,
    end: usize,
    max_atom_bytes: usize,
    atoms: &mut Vec<(usize, usize)>,
) {
    let mut piece_start = start;
    while end - piece_start > max_atom_bytes {
        let mut cut = piece_start + max_atom_bytes;
        while !content.is_char_boundary(cut) {
            cut -= 1;
        }
        atoms.push((piece_start, cut));
        piece_start = cut;
    }
    atoms.push((piece_start, end));
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
        let timeline_chunk_count = chunks
            .iter()
            .filter(|chunk| chunk.chunk_type == "timeline_entry")
            .count();

        assert_eq!(timeline_chunk_count, 5);
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
