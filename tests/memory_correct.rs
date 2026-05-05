use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use quaid::core::conversation::extractor::SlmClient;
use quaid::core::conversation::slm::SlmError;
use quaid::core::db;
use quaid::mcp::server::{
    MemoryCorrectContinueInput, MemoryCorrectInput, MemoryGetInput, MemoryPutInput, QuaidServer,
};
use rmcp::model::{CallToolResult, RawContent};
use rusqlite::{params, Connection};
use serde_json::json;

struct Harness {
    _dir: tempfile::TempDir,
    vault_root: PathBuf,
    server: QuaidServer,
    inspect: Connection,
    slm: Arc<StubSlm>,
}

impl Harness {
    fn new(outputs: Vec<Result<&str, SlmError>>) -> Self {
        let dir = tempfile::TempDir::new().unwrap();
        let db_path = dir.path().join("memory.db");
        let vault_root = dir.path().join("vault");
        std::fs::create_dir_all(&vault_root).unwrap();

        let server_conn = db::open(db_path.to_str().unwrap()).unwrap();
        server_conn
            .execute(
                "UPDATE collections
                 SET root_path = ?1,
                     writable = 1,
                     is_write_target = 1,
                     state = 'active',
                     needs_full_sync = 0
                 WHERE id = 1",
                [vault_root.display().to_string()],
            )
            .unwrap();
        let inspect = db::open(db_path.to_str().unwrap()).unwrap();
        let slm = Arc::new(StubSlm::new(outputs));
        let server = QuaidServer::new_with_slm(server_conn, slm.clone());

        Self {
            _dir: dir,
            vault_root,
            server,
            inspect,
            slm,
        }
    }

    fn create_page(&self, slug: &str, content: &str) {
        self.server
            .memory_put(MemoryPutInput {
                slug: slug.to_string(),
                content: content.to_string(),
                expected_version: None,
                namespace: None,
            })
            .unwrap();
    }

    fn rendered_fact(&self, canonical_slug: &str) -> String {
        let slug = canonical_slug.split_once("::").unwrap().1;
        std::fs::read_to_string(
            self.vault_root
                .join("extracted")
                .join("facts")
                .join(format!("{slug}.md")),
        )
        .unwrap()
    }

    fn get_page(&self, slug: &str) -> serde_json::Value {
        let result = self
            .server
            .memory_get(MemoryGetInput {
                slug: slug.to_string(),
            })
            .unwrap();
        serde_json::from_str(&extract_text(&result)).unwrap()
    }
}

#[derive(Default)]
struct StubSlm {
    outputs: Mutex<VecDeque<Result<String, SlmError>>>,
    calls: Mutex<Vec<String>>,
}

impl StubSlm {
    fn new(outputs: Vec<Result<&str, SlmError>>) -> Self {
        Self {
            outputs: Mutex::new(
                outputs
                    .into_iter()
                    .map(|result| result.map(str::to_string))
                    .collect(),
            ),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl SlmClient for StubSlm {
    fn infer(&self, _alias: &str, prompt: &str, _max_tokens: usize) -> Result<String, SlmError> {
        self.calls.lock().unwrap().push(prompt.to_string());
        self.outputs.lock().unwrap().pop_front().unwrap_or_else(|| {
            Ok("{\"outcome\":\"abandon\",\"reason\":\"no stubbed output\"}".to_string())
        })
    }
}

fn fact_page(summary: &str) -> String {
    format!(
        "{}{}",
        concat!(
            "---\n",
            "title: programming-language\n",
            "type: fact\n",
            "kind: fact\n",
            "about: programming-language\n",
            "session_id: session-1\n",
            "source_turns:\n",
            "  - session-1:1\n",
            "supersedes: null\n",
            "corrected_via: null\n",
            "---\n"
        ),
        summary
    ) + "\n"
}

fn note_page() -> &'static str {
    concat!(
        "---\n",
        "title: regular-note\n",
        "type: note\n",
        "---\n",
        "This is a regular note.\n"
    )
}

fn extract_text(result: &CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|content| match &content.raw {
            RawContent::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

#[test]
fn memory_correct_commits_in_one_shot_when_model_returns_commit() {
    let harness = Harness::new(vec![Ok(
        r#"{"outcome":"commit","fact":{"kind":"fact","about":"programming-language","summary":"Matt now prefers Zig for systems work."}}"#,
    )]);
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );

    let result = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "Update this to Zig.".to_string(),
        })
        .unwrap();

    let payload: serde_json::Value = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(payload["status"], "committed");
    assert_eq!(payload["supersedes"], "default::facts/programming-language");
    let new_slug = payload["new_fact_slug"].as_str().unwrap();
    let rendered = harness.rendered_fact(new_slug);
    assert!(rendered.contains("corrected_via: explicit"));
    assert!(rendered.contains("supersedes: facts/programming-language"));
    assert!(rendered.contains("Matt now prefers Zig for systems work."));
    let new_page = harness.get_page(new_slug);
    assert_eq!(
        new_page["summary"],
        "Matt now prefers Zig for systems work."
    );

    let session_row: (String, u32) = harness
        .inspect
        .query_row(
            "SELECT status, turns_used FROM correction_sessions",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(session_row, ("committed".to_string(), 1));
    assert_eq!(harness.slm.call_count(), 1);
}

#[test]
fn memory_correct_continue_commits_after_a_clarification_round() {
    let harness = Harness::new(vec![
        Ok(r#"{"outcome":"clarify","question":"Do you mean Zig or Go?"}"#),
        Ok(
            r#"{"outcome":"commit","fact":{"kind":"fact","about":"programming-language","summary":"Matt switched from Rust to Zig."}}"#,
        ),
    ]);
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );

    let first = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "That is no longer right.".to_string(),
        })
        .unwrap();
    let first_payload: serde_json::Value = serde_json::from_str(&extract_text(&first)).unwrap();
    assert_eq!(first_payload["status"], "needs_clarification");
    assert_eq!(first_payload["turns_remaining"], 2);

    let correction_id = first_payload["correction_id"].as_str().unwrap().to_string();
    let second = harness
        .server
        .memory_correct_continue(MemoryCorrectContinueInput {
            correction_id: correction_id.clone(),
            response: Some("I mean Zig.".to_string()),
            abandon: None,
        })
        .unwrap();
    let second_payload: serde_json::Value = serde_json::from_str(&extract_text(&second)).unwrap();
    assert_eq!(second_payload["status"], "committed");

    let session_row: (String, u32, String) = harness
        .inspect
        .query_row(
            "SELECT status, turns_used, exchange_log
             FROM correction_sessions
             WHERE correction_id = ?1",
            [&correction_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(session_row.0, "committed");
    assert_eq!(session_row.1, 2);
    let exchange_log: serde_json::Value = serde_json::from_str(&session_row.2).unwrap();
    assert_eq!(exchange_log.as_array().unwrap().len(), 4);
}

#[test]
fn memory_correct_continue_abandons_without_a_second_slm_call_when_user_requests_it() {
    let harness = Harness::new(vec![Ok(
        r#"{"outcome":"clarify","question":"What should the corrected language be?"}"#,
    )]);
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );

    let first = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "This fact is stale.".to_string(),
        })
        .unwrap();
    let correction_id = serde_json::from_str::<serde_json::Value>(&extract_text(&first)).unwrap()
        ["correction_id"]
        .as_str()
        .unwrap()
        .to_string();

    let abandoned = harness
        .server
        .memory_correct_continue(MemoryCorrectContinueInput {
            correction_id: correction_id.clone(),
            response: None,
            abandon: Some(true),
        })
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&extract_text(&abandoned)).unwrap();
    assert_eq!(
        payload,
        json!({"status":"abandoned","reason":"user_requested"})
    );
    assert_eq!(harness.slm.call_count(), 1);

    let status: String = harness
        .inspect
        .query_row(
            "SELECT status FROM correction_sessions WHERE correction_id = ?1",
            [&correction_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "abandoned");
}

#[test]
fn memory_correct_continue_forces_turn_cap_abandon_on_third_non_commit() {
    let harness = Harness::new(vec![
        Ok(r#"{"outcome":"clarify","question":"Do you mean Zig or Go?"}"#),
        Ok(r#"{"outcome":"clarify","question":"Should I update the summary too?"}"#),
        Ok(r#"{"outcome":"clarify","question":"Any other nuance?"}"#),
    ]);
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );

    let first = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "This is stale.".to_string(),
        })
        .unwrap();
    let correction_id = serde_json::from_str::<serde_json::Value>(&extract_text(&first)).unwrap()
        ["correction_id"]
        .as_str()
        .unwrap()
        .to_string();

    let second = harness
        .server
        .memory_correct_continue(MemoryCorrectContinueInput {
            correction_id: correction_id.clone(),
            response: Some("It should say Zig.".to_string()),
            abandon: None,
        })
        .unwrap();
    let second_payload: serde_json::Value = serde_json::from_str(&extract_text(&second)).unwrap();
    assert_eq!(second_payload["status"], "needs_clarification");
    assert_eq!(second_payload["turns_remaining"], 1);

    let third = harness
        .server
        .memory_correct_continue(MemoryCorrectContinueInput {
            correction_id: correction_id.clone(),
            response: Some("No more nuance.".to_string()),
            abandon: None,
        })
        .unwrap();
    let third_payload: serde_json::Value = serde_json::from_str(&extract_text(&third)).unwrap();
    assert_eq!(
        third_payload,
        json!({"status":"abandoned","reason":"turn_cap_reached"})
    );

    let row: (String, u32) = harness
        .inspect
        .query_row(
            "SELECT status, turns_used FROM correction_sessions WHERE correction_id = ?1",
            [&correction_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(row, ("abandoned".to_string(), 3));
}

#[test]
fn memory_correct_continue_rejects_expired_sessions() {
    let harness = Harness::new(vec![Ok(
        r#"{"outcome":"clarify","question":"What should the corrected language be?"}"#,
    )]);
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );

    let first = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "This is stale.".to_string(),
        })
        .unwrap();
    let correction_id = serde_json::from_str::<serde_json::Value>(&extract_text(&first)).unwrap()
        ["correction_id"]
        .as_str()
        .unwrap()
        .to_string();

    harness
        .inspect
        .execute(
            "UPDATE correction_sessions
             SET expires_at = '2000-01-01T00:00:00Z'
             WHERE correction_id = ?1",
            [&correction_id],
        )
        .unwrap();

    let error = harness
        .server
        .memory_correct_continue(MemoryCorrectContinueInput {
            correction_id: correction_id.clone(),
            response: Some("Say Zig.".to_string()),
            abandon: None,
        })
        .unwrap_err();
    assert_eq!(error.code, rmcp::model::ErrorCode(-32009));
    assert!(error.message.contains("expired"));
    assert_eq!(harness.slm.call_count(), 1);

    let status: String = harness
        .inspect
        .query_row(
            "SELECT status FROM correction_sessions WHERE correction_id = ?1",
            [&correction_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "expired");
}

#[test]
fn memory_correct_rejects_non_head_facts() {
    let harness = Harness::new(Vec::new());
    harness.create_page(
        "facts/programming-language",
        &fact_page("Matt prefers Rust."),
    );
    harness.create_page(
        "facts/programming-language-v2",
        &fact_page("Matt prefers Zig."),
    );

    let successor_id: i64 = harness
        .inspect
        .query_row(
            "SELECT id FROM pages WHERE slug = 'facts/programming-language-v2'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    harness
        .inspect
        .execute(
            "UPDATE pages SET superseded_by = ?2 WHERE slug = ?1",
            params!["facts/programming-language", successor_id],
        )
        .unwrap();

    let error = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "facts/programming-language".to_string(),
            correction: "Use Zig.".to_string(),
        })
        .unwrap_err();
    assert_eq!(error.code, rmcp::model::ErrorCode(-32009));
    assert!(error.message.contains("current head"));
    assert_eq!(harness.slm.call_count(), 0);
}

#[test]
fn memory_correct_rejects_non_fact_kind_pages() {
    let harness = Harness::new(Vec::new());
    harness.create_page("notes/regular-note", note_page());

    let error = harness
        .server
        .memory_correct(MemoryCorrectInput {
            fact_slug: "notes/regular-note".to_string(),
            correction: "Try to correct this.".to_string(),
        })
        .unwrap_err();
    assert_eq!(error.code, rmcp::model::ErrorCode(-32002));
    assert!(error.message.contains("KindError"));
    assert_eq!(harness.slm.call_count(), 0);
}
