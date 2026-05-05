use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use thiserror::Error;

use crate::core::types::{
    ConversationFile, ConversationFrontmatter, ConversationStatus, Turn, TurnRole,
};
use crate::core::{collections, namespace};

const HEADING_SEPARATOR: &str = " · ";
const METADATA_FENCE_OPEN: &str = "```json turn-metadata";

#[derive(Debug, Error)]
pub enum ConversationFormatError {
    #[error("conversation read failed: {0}")]
    Io(#[from] std::io::Error),

    #[error("invalid conversation frontmatter: {message}")]
    InvalidFrontmatter { message: String },

    #[error("invalid turn block near line {line}: {message}")]
    InvalidTurnBlock { line: usize, message: String },

    #[error("invalid turn metadata near line {line}: {message}")]
    InvalidMetadata { line: usize, message: String },

    #[error("invalid timestamp: {timestamp}")]
    InvalidTimestamp { timestamp: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryLocation {
    VaultSubdir,
    DedicatedCollection,
}

impl MemoryLocation {
    pub fn from_config(value: &str) -> Result<Self, ConversationFormatError> {
        match value {
            "vault-subdir" => Ok(Self::VaultSubdir),
            "dedicated-collection" => Ok(Self::DedicatedCollection),
            other => Err(ConversationFormatError::InvalidFrontmatter {
                message: format!("unsupported memory.location value: {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationPathInfo {
    pub relative_path: PathBuf,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConversationPath {
    pub namespace: Option<String>,
    pub date: String,
    pub session_id: String,
}

pub fn parse(path: &Path) -> Result<ConversationFile, ConversationFormatError> {
    let raw = fs::read_to_string(path)?;
    parse_str(&raw)
}

pub fn parse_str(raw: &str) -> Result<ConversationFile, ConversationFormatError> {
    let lines = normalize_lines(raw);
    let (frontmatter, mut index) = parse_frontmatter(&lines)?;
    let mut turns = Vec::new();

    while index < lines.len() {
        while index < lines.len() && lines[index].trim().is_empty() {
            index += 1;
        }
        if index >= lines.len() {
            break;
        }
        let (turn, next_index) = parse_turn_block(&lines, index)?;
        turns.push(turn);
        index = next_index;
    }

    Ok(ConversationFile { frontmatter, turns })
}

pub fn render(file: &ConversationFile) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str("type: conversation\n");
    out.push_str("session_id: ");
    out.push_str(&file.frontmatter.session_id);
    out.push('\n');
    out.push_str("date: ");
    out.push_str(&file.frontmatter.date);
    out.push('\n');
    out.push_str("started_at: ");
    out.push_str(&file.frontmatter.started_at);
    out.push('\n');
    out.push_str("status: ");
    out.push_str(file.frontmatter.status.as_str());
    out.push('\n');
    if let Some(value) = file.frontmatter.closed_at.as_deref() {
        out.push_str("closed_at: ");
        out.push_str(value);
        out.push('\n');
    }
    out.push_str("last_extracted_at: ");
    if let Some(value) = file.frontmatter.last_extracted_at.as_deref() {
        out.push_str(value);
    } else {
        out.push_str("null");
    }
    out.push('\n');
    out.push_str("last_extracted_turn: ");
    out.push_str(&file.frontmatter.last_extracted_turn.to_string());
    out.push_str("\n---\n");

    if !file.turns.is_empty() {
        out.push('\n');
    }

    for (index, turn) in file.turns.iter().enumerate() {
        if index > 0 {
            out.push_str("\n---\n\n");
        }
        out.push_str(&render_turn_block(turn));
    }

    out
}

pub fn render_turn_block(turn: &Turn) -> String {
    let mut out = String::new();
    out.push_str("## Turn ");
    out.push_str(&turn.ordinal.to_string());
    out.push_str(HEADING_SEPARATOR);
    out.push_str(turn.role.as_str());
    out.push_str(HEADING_SEPARATOR);
    out.push_str(&turn.timestamp);
    out.push_str("\n\n");
    out.push_str(&turn.content);
    if !turn.content.ends_with('\n') {
        out.push('\n');
    }
    if let Some(metadata) = &turn.metadata {
        out.push('\n');
        out.push_str(METADATA_FENCE_OPEN);
        out.push('\n');
        out.push_str(&serde_json::to_string_pretty(metadata).expect("serialize metadata"));
        out.push_str("\n```\n");
    }
    out
}

pub fn conversation_path_for(
    namespace: Option<&str>,
    session_id: &str,
    timestamp: &str,
) -> Result<ConversationPathInfo, ConversationFormatError> {
    let date = date_from_timestamp(timestamp)?;
    let mut relative_path = PathBuf::new();
    if let Some(namespace) = namespace.filter(|value| !value.is_empty()) {
        relative_path.push(namespace);
    }
    relative_path.push("conversations");
    relative_path.push(&date);
    relative_path.push(format!("{session_id}.md"));

    Ok(ConversationPathInfo {
        relative_path,
        date,
    })
}

pub fn parse_relative_conversation_path(
    path: &str,
) -> Result<ParsedConversationPath, ConversationFormatError> {
    collections::validate_relative_path(path).map_err(|error| {
        ConversationFormatError::InvalidFrontmatter {
            message: error.to_string(),
        }
    })?;

    let components = path.split('/').collect::<Vec<_>>();
    let (namespace, conversations_idx) = match components.as_slice() {
        ["conversations", _, _] => (None, 0),
        [namespace, "conversations", _, _] => {
            namespace::validate_optional_namespace(Some(namespace)).map_err(|error| {
                ConversationFormatError::InvalidFrontmatter {
                    message: error.to_string(),
                }
            })?;
            (Some((*namespace).to_string()), 1)
        }
        _ => {
            return Err(ConversationFormatError::InvalidFrontmatter {
                message: format!(
                    "conversation path must match [<namespace>/]conversations/<date>/<session>.md, found `{path}`"
                ),
            });
        }
    };

    let date = components[conversations_idx + 1].to_string();
    validate_date_segment(&date)?;

    let file_name = components[conversations_idx + 2];
    let session_id = file_name
        .strip_suffix(".md")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
            message: format!("conversation path must end with `<session>.md`, found `{path}`"),
        })?
        .to_string();
    collections::validate_relative_path(&session_id).map_err(|error| {
        ConversationFormatError::InvalidFrontmatter {
            message: error.to_string(),
        }
    })?;

    Ok(ParsedConversationPath {
        namespace,
        date,
        session_id,
    })
}

pub fn date_from_timestamp(timestamp: &str) -> Result<String, ConversationFormatError> {
    let bytes = timestamp.as_bytes();
    if timestamp.len() < 10
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return Err(ConversationFormatError::InvalidTimestamp {
            timestamp: timestamp.to_owned(),
        });
    }
    Ok(timestamp[..10].to_owned())
}

fn validate_date_segment(date: &str) -> Result<(), ConversationFormatError> {
    let bytes = date.as_bytes();
    if date.len() != 10
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        return Err(ConversationFormatError::InvalidFrontmatter {
            message: format!("invalid conversation date segment: {date}"),
        });
    }
    Ok(())
}

fn parse_frontmatter(
    lines: &[String],
) -> Result<(ConversationFrontmatter, usize), ConversationFormatError> {
    if lines.first().map(String::as_str) != Some("---") {
        return Err(ConversationFormatError::InvalidFrontmatter {
            message: "missing opening ---".to_owned(),
        });
    }

    let mut index = 1;
    let mut file_type = None;
    let mut session_id = None;
    let mut date = None;
    let mut started_at = None;
    let mut status = None;
    let mut closed_at = None;
    let mut last_extracted_at = None;
    let mut last_extracted_turn = None;

    while index < lines.len() {
        let line = lines[index].trim_end();
        if line == "---" {
            index += 1;
            break;
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(ConversationFormatError::InvalidFrontmatter {
                message: format!("expected `key: value`, found `{line}`"),
            });
        };
        let value = value.trim().to_owned();
        match key.trim() {
            "type" => file_type = Some(value),
            "session_id" => session_id = Some(value),
            "date" => date = Some(value),
            "started_at" => started_at = Some(value),
            "status" => status = Some(value),
            "closed_at" => {
                closed_at = if value.is_empty() || value == "null" {
                    None
                } else {
                    Some(value)
                };
            }
            "last_extracted_at" => {
                last_extracted_at = if value.is_empty() || value == "null" {
                    None
                } else {
                    Some(value)
                };
            }
            "last_extracted_turn" => {
                let parsed = value.parse::<i64>().map_err(|_| {
                    ConversationFormatError::InvalidFrontmatter {
                        message: format!("last_extracted_turn must be an integer, found `{value}`"),
                    }
                })?;
                last_extracted_turn = Some(parsed);
            }
            _ => {}
        }
        index += 1;
    }

    let file_type = file_type.ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
        message: "missing type".to_owned(),
    })?;
    if file_type != "conversation" {
        return Err(ConversationFormatError::InvalidFrontmatter {
            message: format!("expected type=conversation, found `{file_type}`"),
        });
    }

    let status = status
        .ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
            message: "missing status".to_owned(),
        })?
        .parse::<ConversationStatus>()
        .map_err(|message| ConversationFormatError::InvalidFrontmatter { message })?;

    Ok((
        ConversationFrontmatter {
            file_type,
            session_id: session_id.ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
                message: "missing session_id".to_owned(),
            })?,
            date: date.ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
                message: "missing date".to_owned(),
            })?,
            started_at: started_at.ok_or_else(|| ConversationFormatError::InvalidFrontmatter {
                message: "missing started_at".to_owned(),
            })?,
            status,
            closed_at,
            last_extracted_at,
            last_extracted_turn: last_extracted_turn.ok_or_else(|| {
                ConversationFormatError::InvalidFrontmatter {
                    message: "missing last_extracted_turn".to_owned(),
                }
            })?,
        },
        index,
    ))
}

fn parse_turn_block(
    lines: &[String],
    start: usize,
) -> Result<(Turn, usize), ConversationFormatError> {
    let header = lines[start].trim_end();
    let Some(header) = header.strip_prefix("## Turn ") else {
        return Err(ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "expected `## Turn N · role · timestamp` heading".to_owned(),
        });
    };
    let mut header_parts = header.split(HEADING_SEPARATOR);
    let ordinal = header_parts
        .next()
        .ok_or_else(|| ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "missing turn ordinal".to_owned(),
        })?
        .parse::<i64>()
        .map_err(|_| ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "turn ordinal must be an integer".to_owned(),
        })?;
    let role = header_parts
        .next()
        .ok_or_else(|| ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "missing turn role".to_owned(),
        })?
        .parse::<TurnRole>()
        .map_err(|message| ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message,
        })?;
    let timestamp = header_parts
        .next()
        .ok_or_else(|| ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "missing turn timestamp".to_owned(),
        })?
        .to_owned();
    if header_parts.next().is_some() {
        return Err(ConversationFormatError::InvalidTurnBlock {
            line: start + 1,
            message: "unexpected extra heading fields".to_owned(),
        });
    }

    let mut end = start + 1;
    let mut in_code_fence = false;
    while end < lines.len() {
        let line = lines[end].trim_end();
        if line.starts_with("```") {
            in_code_fence = !in_code_fence;
        }
        if !in_code_fence && line == "---" {
            break;
        }
        end += 1;
    }

    let mut block_lines = lines[start + 1..end].to_vec();
    while block_lines
        .first()
        .map(|line| line.trim().is_empty())
        .unwrap_or(false)
    {
        block_lines.remove(0);
    }

    let (content, metadata) = split_content_and_metadata(&block_lines, start + 2)?;
    let next_index = if end < lines.len() { end + 1 } else { end };

    Ok((
        Turn {
            ordinal,
            role,
            timestamp,
            content,
            metadata,
        },
        next_index,
    ))
}

fn split_content_and_metadata(
    lines: &[String],
    base_line: usize,
) -> Result<(String, Option<JsonValue>), ConversationFormatError> {
    let metadata_end = lines.iter().rposition(|line| !line.trim().is_empty());
    let metadata_start = metadata_end
        .filter(|end| lines[*end].trim() == "```")
        .and_then(|end| {
            lines[..end]
                .iter()
                .enumerate()
                .rev()
                .find_map(|(index, line)| (line.trim() == METADATA_FENCE_OPEN).then_some(index))
                .map(|start| (start, end))
        });

    let (content_lines, metadata) = if let Some((start, end)) = metadata_start {
        let metadata_json = lines[start + 1..end].join("\n");
        let metadata = serde_json::from_str::<JsonValue>(&metadata_json).map_err(|error| {
            ConversationFormatError::InvalidMetadata {
                line: base_line + start,
                message: error.to_string(),
            }
        })?;
        (&lines[..start], Some(metadata))
    } else {
        (lines, None)
    };

    let mut content_lines = content_lines.to_vec();
    while content_lines
        .last()
        .map(|line| line.trim().is_empty())
        .unwrap_or(false)
    {
        content_lines.pop();
    }

    Ok((content_lines.join("\n"), metadata))
}

fn normalize_lines(raw: &str) -> Vec<String> {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file() -> ConversationFile {
        ConversationFile {
            frontmatter: ConversationFrontmatter {
                file_type: "conversation".to_owned(),
                session_id: "session-1".to_owned(),
                date: "2026-05-03".to_owned(),
                started_at: "2026-05-03T09:14:22Z".to_owned(),
                status: ConversationStatus::Open,
                closed_at: None,
                last_extracted_at: Some("2026-05-03T10:30:18Z".to_owned()),
                last_extracted_turn: 2,
            },
            turns: vec![
                Turn {
                    ordinal: 1,
                    role: TurnRole::User,
                    timestamp: "2026-05-03T09:14:22Z".to_owned(),
                    content: "hello".to_owned(),
                    metadata: None,
                },
                Turn {
                    ordinal: 2,
                    role: TurnRole::Assistant,
                    timestamp: "2026-05-03T09:14:30Z".to_owned(),
                    content: "world".to_owned(),
                    metadata: Some(serde_json::json!({"tool_name":"bash","importance":"high"})),
                },
            ],
        }
    }

    #[test]
    fn render_and_parse_round_trip_preserves_canonical_shape() {
        let file = sample_file();
        let rendered = render(&file);
        let parsed = parse_str(&rendered).expect("parse rendered file");

        assert_eq!(parsed, file);
        assert_eq!(render(&parsed), rendered);
    }

    #[test]
    fn parse_preserves_frontmatter_cursor_fields() {
        let rendered = render(&sample_file());
        let parsed = parse_str(&rendered).expect("parse rendered file");

        assert_eq!(parsed.frontmatter.last_extracted_turn, 2);
        assert_eq!(
            parsed.frontmatter.last_extracted_at.as_deref(),
            Some("2026-05-03T10:30:18Z")
        );
    }

    #[test]
    fn conversation_path_for_nests_namespace_when_present() {
        let path = conversation_path_for(Some("alpha"), "session-1", "2026-05-04T00:01:00Z")
            .expect("path info");

        assert_eq!(
            path.relative_path,
            PathBuf::from("alpha")
                .join("conversations")
                .join("2026-05-04")
                .join("session-1.md")
        );
    }

    #[test]
    fn parse_relative_conversation_path_extracts_namespace_and_session() {
        let parsed =
            parse_relative_conversation_path("alpha/conversations/2026-05-04/session-1.md")
                .expect("valid relative conversation path");

        assert_eq!(parsed.namespace.as_deref(), Some("alpha"));
        assert_eq!(parsed.date, "2026-05-04");
        assert_eq!(parsed.session_id, "session-1");
    }

    #[test]
    fn parse_relative_conversation_path_rejects_missing_date_segment() {
        let error = parse_relative_conversation_path("alpha/conversations/session-1.md")
            .expect_err("missing date must fail");

        assert!(error.to_string().contains("conversation path must match"));
    }

    #[test]
    fn parse_reports_actionable_error_for_malformed_turn_block() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: \n\
last_extracted_turn: 0\n\
---\n\n\
## Turn nope · user · 2026-05-03T09:14:22Z\n\n\
body\n";

        let error = parse_str(input).expect_err("malformed ordinal should fail");
        assert!(error.to_string().contains("line"));
        assert!(error
            .to_string()
            .contains("turn ordinal must be an integer"));
    }

    #[test]
    fn parse_rejects_non_numeric_last_extracted_turn() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: null\n\
last_extracted_turn: nope\n\
---\n";

        let error = parse_str(input).expect_err("non-numeric cursor should fail");

        assert!(error
            .to_string()
            .contains("last_extracted_turn must be an integer"));
    }

    #[test]
    fn parse_rejects_turn_heading_with_extra_fields() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: null\n\
last_extracted_turn: 0\n\
---\n\n\
## Turn 1 · user · 2026-05-03T09:14:22Z · extra\n\n\
body\n";

        let error = parse_str(input).expect_err("extra heading fields should fail");

        assert!(error
            .to_string()
            .contains("unexpected extra heading fields"));
    }

    #[test]
    fn render_outputs_null_for_missing_last_extracted_at() {
        let mut file = sample_file();
        file.frontmatter.last_extracted_at = None;

        let rendered = render(&file);

        assert!(rendered.contains("last_extracted_at: null"));
    }

    #[test]
    fn parse_rejects_missing_frontmatter_boundary() {
        let error = parse_str("type: conversation\n").expect_err("missing boundary should fail");

        assert!(error.to_string().contains("missing opening ---"));
    }

    #[test]
    fn parse_handles_non_json_code_fences_inside_turn_content() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: null\n\
last_extracted_turn: 0\n\
---\n\n\
## Turn 1 · user · 2026-05-03T09:14:22Z\n\n\
```bash\n\
echo hi\n\
```\n\n\
---\n\n\
## Turn 2 · assistant · 2026-05-03T09:14:30Z\n\n\
done\n";

        let parsed = parse_str(input).expect("parse fenced content");

        assert_eq!(parsed.turns.len(), 2);
        assert!(parsed.turns[0].content.contains("```bash"));
        assert_eq!(parsed.turns[1].content, "done");
    }

    #[test]
    fn parse_reports_invalid_metadata_json() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: null\n\
last_extracted_turn: 0\n\
---\n\n\
## Turn 1 · user · 2026-05-03T09:14:22Z\n\n\
hello\n\n\
```json turn-metadata\n\
{oops}\n\
```\n";

        let error = parse_str(input).expect_err("invalid metadata should fail");

        assert!(error.to_string().contains("invalid turn metadata"));
    }

    #[test]
    fn parse_keeps_trailing_json_code_fence_in_turn_content() {
        let input = "---\n\
type: conversation\n\
session_id: session-1\n\
date: 2026-05-03\n\
started_at: 2026-05-03T09:14:22Z\n\
status: open\n\
last_extracted_at: null\n\
last_extracted_turn: 0\n\
---\n\n\
## Turn 1 · user · 2026-05-03T09:14:22Z\n\n\
Here is the payload.\n\n\
```json\n\
{\n\
  \"importance\": \"high\"\n\
}\n\
```\n";

        let parsed = parse_str(input).expect("parse trailing json content");

        assert_eq!(parsed.turns[0].metadata, None);
        assert!(parsed.turns[0].content.contains("```json"));
        assert_eq!(render(&parsed), input);
    }

    #[test]
    fn date_from_timestamp_rejects_invalid_prefix() {
        let error = date_from_timestamp("20260503T09:14:22Z").expect_err("bad timestamp");

        assert!(error.to_string().contains("invalid timestamp"));
    }
}
