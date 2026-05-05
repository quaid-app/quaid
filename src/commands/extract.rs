use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use clap::Args;
use rusqlite::Connection;

use crate::core::conversation::{format, queue, turn_writer};
use crate::core::types::ExtractionTriggerKind;

#[derive(Args, Clone, Debug, PartialEq, Eq)]
pub struct ExtractArgs {
    /// Re-extract a single session id
    session_id: Option<String>,
    /// Re-extract every known session
    #[arg(long, conflicts_with = "session_id")]
    all: bool,
    /// Restrict `--all` to sessions with day-files dated on or after YYYY-MM-DD
    #[arg(long, requires = "all", value_name = "date")]
    since: Option<String>,
    /// Reset conversation cursors to turn 0 before enqueueing
    #[arg(long, requires = "session_id", conflicts_with = "all")]
    force: bool,
}

pub fn run(db: &Connection, args: ExtractArgs) -> Result<()> {
    if !args.all && args.session_id.is_none() {
        bail!("provide <session-id> or --all");
    }

    let root = turn_writer::resolve_memory_root(db)?;
    let sessions = discover_sessions(&root.root_path)?;
    let since = args.since.as_deref().map(validate_date).transpose()?;

    let targets = if args.all {
        sessions
            .into_values()
            .filter(|session| {
                since.as_deref().map_or(true, |cutoff| {
                    session
                        .day_files
                        .iter()
                        .any(|file| file.date.as_str() >= cutoff)
                })
            })
            .collect::<Vec<_>>()
    } else {
        select_sessions(
            &sessions,
            args.session_id
                .as_deref()
                .expect("session id when not --all"),
        )?
    };

    if targets.is_empty() {
        if args.all {
            bail!("no sessions matched the requested extract scope");
        }
        bail!(
            "session `{}` was not found in conversations/",
            args.session_id.as_deref().unwrap_or_default()
        );
    }

    let scheduled_for = queue::current_timestamp(db)?;
    let mut enqueued = Vec::with_capacity(targets.len());
    for target in &targets {
        if args.force {
            reset_cursors(&root.root_path, target)?;
        }
        queue::enqueue(
            db,
            &target.queue_session_id,
            &target.latest_relative_path,
            ExtractionTriggerKind::Manual,
            &scheduled_for,
        )?;
        enqueued.push(target.display_id.clone());
    }

    println!(
        "Enqueued manual extraction for {} session(s):",
        enqueued.len()
    );
    for session_id in &enqueued {
        println!("  - {session_id}");
    }
    println!("Track progress with `quaid extraction status`.");
    Ok(())
}

fn select_sessions(
    sessions: &BTreeMap<String, SessionTarget>,
    selector: &str,
) -> Result<Vec<SessionTarget>> {
    let matches = sessions
        .values()
        .filter(|session| session.session_id == selector || session.display_id == selector)
        .cloned()
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Ok(matches);
    }
    if matches.len() > 1 {
        let options = matches
            .iter()
            .map(|session| session.display_id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!("session selector `{selector}` is ambiguous; matches: {options}");
    }
    Ok(matches)
}

fn discover_sessions(root: &Path) -> Result<BTreeMap<String, SessionTarget>> {
    let mut sessions = BTreeMap::<String, SessionTarget>::new();
    for relative_path in conversation_paths(root)? {
        let parsed = format::parse_relative_conversation_path(&relative_path)?;
        let key = queue::session_queue_key(parsed.namespace.as_deref(), &parsed.session_id);
        let display_id = display_id(parsed.namespace.as_deref(), &parsed.session_id);
        let day_file = SessionDayFile {
            date: parsed.date.clone(),
            relative_path: relative_path.clone(),
        };
        let latest_relative_path = relative_path.clone();
        let entry = sessions
            .entry(key.clone())
            .or_insert_with(|| SessionTarget {
                queue_session_id: key,
                display_id,
                session_id: parsed.session_id.clone(),
                latest_relative_path,
                day_files: Vec::new(),
            });
        if day_file.date > date_from_relative_path(&entry.latest_relative_path)? {
            entry.latest_relative_path = relative_path.clone();
        }
        entry.day_files.push(day_file);
    }

    for session in sessions.values_mut() {
        session
            .day_files
            .sort_by(|left, right| left.date.cmp(&right.date));
    }
    Ok(sessions)
}

fn reset_cursors(root: &Path, session: &SessionTarget) -> Result<()> {
    for day_file in &session.day_files {
        let absolute = root.join(slash_path_to_platform(&day_file.relative_path));
        let mut conversation = format::parse(&absolute)?;
        conversation.frontmatter.last_extracted_turn = 0;
        conversation.frontmatter.last_extracted_at = None;
        fs::write(&absolute, format::render(&conversation))?;
    }
    Ok(())
}

fn conversation_paths(root: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for base in conversation_roots(root)? {
        let conversations_dir = root.join(&base);
        if !conversations_dir.is_dir() {
            continue;
        }
        for date_entry in fs::read_dir(&conversations_dir)? {
            let date_entry = date_entry?;
            let date_path = date_entry.path();
            if !date_path.is_dir() {
                continue;
            }
            for file_entry in fs::read_dir(&date_path)? {
                let file_entry = file_entry?;
                let file_path = file_entry.path();
                if !file_path.is_file()
                    || file_path.extension().and_then(|ext| ext.to_str()) != Some("md")
                {
                    continue;
                }
                paths.push(
                    file_path
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn conversation_roots(root: &Path) -> Result<Vec<PathBuf>> {
    let mut roots = vec![PathBuf::from("conversations")];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(namespace) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if namespace == "conversations" {
            continue;
        }
        let relative = PathBuf::from(namespace).join("conversations");
        if root.join(&relative).is_dir() {
            roots.push(relative);
        }
    }
    roots.sort();
    Ok(roots)
}

fn validate_date(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    if value.len() != 10
        || bytes.get(4) != Some(&b'-')
        || bytes.get(7) != Some(&b'-')
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[5..7].iter().all(u8::is_ascii_digit)
        || !bytes[8..10].iter().all(u8::is_ascii_digit)
    {
        bail!("invalid --since date `{value}`; expected YYYY-MM-DD");
    }
    Ok(value.to_owned())
}

fn date_from_relative_path(path: &str) -> Result<String> {
    Ok(format::parse_relative_conversation_path(path)?.date)
}

fn display_id(namespace: Option<&str>, session_id: &str) -> String {
    match namespace.filter(|value| !value.is_empty()) {
        Some(namespace) => format!("{namespace}/{session_id}"),
        None => session_id.to_owned(),
    }
}

fn slash_path_to_platform(path: &str) -> PathBuf {
    if std::path::MAIN_SEPARATOR == '/' {
        PathBuf::from(path)
    } else {
        PathBuf::from(path.replace('/', std::path::MAIN_SEPARATOR_STR))
    }
}

#[derive(Clone, Debug)]
struct SessionTarget {
    queue_session_id: String,
    display_id: String,
    session_id: String,
    latest_relative_path: String,
    day_files: Vec<SessionDayFile>,
}

#[derive(Clone, Debug)]
struct SessionDayFile {
    date: String,
    relative_path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_date_accepts_iso_day() {
        assert_eq!(validate_date("2026-05-05").unwrap(), "2026-05-05");
    }

    #[test]
    fn validate_date_rejects_invalid_input() {
        let error = validate_date("20260505").unwrap_err();
        assert!(error.to_string().contains("expected YYYY-MM-DD"));
    }
}
