#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "test fixtures legitimately panic on setup failure and print diagnostics; per-site #[expect] would generate noise across thousands of test sites"
)]

//! Public-API integration tests for `quaid::mcp::server` covering the
//! read-side tool surfaces — `memory_query`, `memory_search`, and
//! `memory_list`. Exercises hybrid-search behavior, FTS5 sanitization,
//! collection filtering and write-target defaults, the `auto`
//! depth-expansion contract, and gap logging on weak results.

#[path = "common/mcp_harness.rs"]
mod harness;
use harness::{
    create_page, create_page_in_collection, extract_query_results, extract_text, insert_collection,
    open_test_db, set_collection_state,
};
use quaid::core::conversation::turn_writer;
use quaid::mcp::server::{
    MemoryLinkInput, MemoryListInput, MemoryPutInput, MemoryQueryInput, MemorySearchInput,
    QuaidServer,
};
use rmcp::model::ErrorCode;

#[test]
fn memory_query_auto_depth_expands_linked_results() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "concepts/root",
        "---\ntitle: Root\ntype: concept\n---\nalpha anchor\n",
    );
    create_page(
        &server,
        "concepts/child",
        "---\ntitle: Child\ntype: concept\n---\nlinked expansion result\n",
    );
    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "concepts/root".to_string(),
            to_slug: "concepts/child".to_string(),
            relationship: "related".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let result = server
        .memory_query(MemoryQueryInput {
            query: "alpha".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: Some(1),
            depth: Some("auto".to_string()),
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert!(rows
        .iter()
        .any(|row| row["slug"] == "default::concepts/child"));
}

#[test]
fn memory_query_auto_depth_does_not_expand_across_collections() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);

    // Anchor page in "default" — will match the query
    create_page(
        &server,
        "concepts/anchor",
        "---\ntitle: Anchor\ntype: concept\n---\ncross collection fence anchor\n",
    );
    // Outside page in "work" — linked from anchor but must NOT appear
    create_page_in_collection(
        &server,
        "work",
        "concepts/outside",
        "---\ntitle: Outside\ntype: concept\n---\nshould not appear in filtered results\n",
    );
    // Cross-collection link: default::concepts/anchor -> work::concepts/outside
    server
        .memory_link(MemoryLinkInput {
            namespace: None,
            from_slug: "default::concepts/anchor".to_string(),
            to_slug: "work::concepts/outside".to_string(),
            relationship: "related".to_string(),
            valid_from: None,
            valid_until: None,
        })
        .unwrap();

    let result = server
        .memory_query(MemoryQueryInput {
            query: "cross collection fence anchor".to_string(),
            collection: Some("default".to_string()),
            namespace: None,
            wing: None,
            limit: Some(5),
            depth: Some("auto".to_string()),
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert!(
        !rows
            .iter()
            .any(|row| row["slug"] == "work::concepts/outside"),
        "depth=auto expansion must not cross into a different collection: got {rows:?}"
    );
}

#[test]
fn memory_query_explicit_collection_filter_returns_only_named_collection() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/default-hit",
        "---\ntitle: Default Hit\ntype: note\n---\nsemantic overlap on robotics leadership\n",
    );
    create_page_in_collection(
        &server,
        "work",
        "notes/work-hit",
        "---\ntitle: Work Hit\ntype: note\n---\nsemantic overlap on robotics leadership\n",
    );

    let result = server
        .memory_query(MemoryQueryInput {
            query: "robotics leadership".to_string(),
            collection: Some("work".to_string()),
            namespace: None,
            wing: None,
            limit: None,
            depth: None,
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "work::notes/work-hit");
}

#[test]
fn memory_query_defaults_to_write_target_when_multiple_collections_are_active() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/default-target",
        "---\ntitle: Default Target\ntype: note\n---\nshared semantic marker\n",
    );
    create_page_in_collection(
        &server,
        "work",
        "notes/work-target",
        "---\ntitle: Work Target\ntype: note\n---\nshared semantic marker\n",
    );

    let result = server
        .memory_query(MemoryQueryInput {
            query: "shared semantic marker".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            depth: None,
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "default::notes/default-target");
}

#[test]
fn memory_query_defaults_to_memory_collection_when_dedicated_memory_location_is_enabled() {
    let (_dir, conn) = open_test_db();
    conn.execute(
        "UPDATE config SET value = 'dedicated-collection' WHERE key = 'memory.location'",
        [],
    )
    .unwrap();
    let memory_root = turn_writer::resolve_memory_root(&conn).unwrap();
    conn.execute(
        "UPDATE collections SET needs_full_sync = 0 WHERE id = ?1",
        [memory_root.collection_id],
    )
    .unwrap();
    let server = QuaidServer::new(conn);
    server
        .memory_put(MemoryPutInput {
            slug: format!("{}::notes/memory-target", memory_root.collection_name),
            content: "---\ntitle: Memory Target\ntype: note\n---\nshared semantic marker\n"
                .to_string(),
            expected_version: None,
            namespace: Some("q0000".to_string()),
        })
        .unwrap();

    let result = server
        .memory_query(MemoryQueryInput {
            query: "shared semantic marker".to_string(),
            collection: None,
            namespace: Some("q0000".to_string()),
            wing: None,
            limit: None,
            depth: None,
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["slug"],
        format!("{}::notes/memory-target", memory_root.collection_name)
    );
}

#[test]
fn memory_search_returns_matching_pages() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme builds fundraising software.\n",
    );

    let result = server
        .memory_search(MemorySearchInput {
            query: "fundraising".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows[0]["slug"], "default::companies/acme");
}

#[test]
fn read_responses_surface_pending_embedding_jobs_hint() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    // A CLI-style write through the MCP put enqueues an embedding job that no
    // daemon has drained, so the read tools must warn about the stale queue.
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme builds fundraising software.\n",
    );

    let search = server
        .memory_search(MemorySearchInput {
            query: "fundraising".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
        })
        .unwrap();
    let envelope: serde_json::Value = serde_json::from_str(&extract_text(&search)).unwrap();
    assert!(
        envelope["pending_embedding_jobs"].as_i64().unwrap_or(0) >= 1,
        "memory_search must surface the pending-embedding-jobs hint when the \
         queue is non-empty: {envelope}"
    );

    let query = server
        .memory_query(MemoryQueryInput {
            query: "fundraising".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            depth: None,
            include_superseded: None,
        })
        .unwrap();
    let query_envelope: serde_json::Value = serde_json::from_str(&extract_text(&query)).unwrap();
    assert!(
        query_envelope["pending_embedding_jobs"]
            .as_i64()
            .unwrap_or(0)
            >= 1,
        "memory_query must surface the pending-embedding-jobs hint: {query_envelope}"
    );

    // With an empty queue the hint is omitted entirely.
    let empty_dir = tempfile::TempDir::new().unwrap();
    let empty_conn =
        quaid::core::db::open(empty_dir.path().join("memory.db").to_str().unwrap()).unwrap();
    let empty_server = QuaidServer::new(empty_conn);
    let empty = empty_server
        .memory_search(MemorySearchInput {
            query: "nothing".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
        })
        .unwrap();
    let empty_envelope: serde_json::Value = serde_json::from_str(&extract_text(&empty)).unwrap();
    assert!(
        empty_envelope.get("pending_embedding_jobs").is_none(),
        "the staleness hint must be omitted when the queue is empty: {empty_envelope}"
    );
}

#[test]
fn memory_search_defaults_to_memory_collection_when_dedicated_memory_location_is_enabled() {
    let (_dir, conn) = open_test_db();
    conn.execute(
        "UPDATE config SET value = 'dedicated-collection' WHERE key = 'memory.location'",
        [],
    )
    .unwrap();
    let memory_root = turn_writer::resolve_memory_root(&conn).unwrap();
    conn.execute(
        "UPDATE collections SET needs_full_sync = 0 WHERE id = ?1",
        [memory_root.collection_id],
    )
    .unwrap();
    let server = QuaidServer::new(conn);
    server
        .memory_put(MemoryPutInput {
            slug: format!("{}::notes/memory-hit", memory_root.collection_name),
            content: "---\ntitle: Memory Hit\ntype: note\n---\nshared needle\n".to_string(),
            expected_version: None,
            namespace: Some("q0000".to_string()),
        })
        .unwrap();

    let result = server
        .memory_search(MemorySearchInput {
            query: "shared".to_string(),
            collection: None,
            namespace: Some("q0000".to_string()),
            wing: None,
            limit: None,
            include_superseded: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["slug"],
        format!("{}::notes/memory-hit", memory_root.collection_name)
    );
}

#[test]
fn memory_search_natural_language_question_mark_returns_valid_response() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    // '?' would be invalid FTS5 syntax if passed raw — memory_search must sanitize.
    let result = server.memory_search(MemorySearchInput {
        query: "what is CLARITY?".to_string(),
        collection: None,
        namespace: None,
        wing: None,
        limit: None,
        include_superseded: None,
        relevance_floor: None,
        max_chunks_per_doc: None,
        redact: None,
    });

    assert!(
        result.is_ok(),
        "memory_search with '?' must not return an MCP error: {result:?}"
    );
    // The response content must parse as a JSON envelope whose `results` is an
    // array (empty or populated).
    let text = extract_text(&result.unwrap());
    let parsed: serde_json::Value =
        serde_json::from_str(&text).expect("memory_search response must be valid JSON");
    assert!(
        parsed
            .get("results")
            .is_some_and(serde_json::Value::is_array),
        "memory_search response must carry a `results` array"
    );
}

#[test]
fn memory_search_explicit_collection_filter_returns_only_named_collection() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/default-hit",
        "---\ntitle: Default Hit\ntype: note\n---\nshared needle\n",
    );
    create_page_in_collection(
        &server,
        "work",
        "notes/work-hit",
        "---\ntitle: Work Hit\ntype: note\n---\nshared needle\n",
    );

    let result = server
        .memory_search(MemorySearchInput {
            query: "shared".to_string(),
            collection: Some("work".to_string()),
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "work::notes/work-hit");
}

#[test]
fn memory_search_defaults_to_single_active_collection() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    set_collection_state(&conn, "default", "detached");
    let server = QuaidServer::new(conn);
    create_page_in_collection(
        &server,
        "work",
        "notes/only-active",
        "---\ntitle: Only Active\ntype: note\n---\nsole active marker\n",
    );

    let result = server
        .memory_search(MemorySearchInput {
            query: "sole".to_string(),
            collection: None,
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap();

    let rows = extract_query_results(&result);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "work::notes/only-active");
}

#[test]
fn memory_list_applies_wing_and_type_filters() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice\ntype: person\n---\nAlice\n",
    );
    create_page(
        &server,
        "companies/acme",
        "---\ntitle: Acme\ntype: company\n---\nAcme\n",
    );

    let result = server
        .memory_list(MemoryListInput {
            collection: None,
            namespace: None,
            wing: Some("people".to_string()),
            page_type: Some("person".to_string()),
            limit: None,
        })
        .unwrap();

    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "default::people/alice");
}

#[test]
fn memory_list_explicit_collection_filter_returns_only_named_collection() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "people/alice",
        "---\ntitle: Alice Default\ntype: person\n---\nDefault Alice\n",
    );
    create_page_in_collection(
        &server,
        "work",
        "people/alice",
        "---\ntitle: Alice Work\ntype: person\n---\nWork Alice\n",
    );

    let result = server
        .memory_list(MemoryListInput {
            collection: Some("work".to_string()),
            namespace: None,
            wing: Some("people".to_string()),
            page_type: Some("person".to_string()),
            limit: None,
        })
        .unwrap();

    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "work::people/alice");
}

#[test]
fn memory_list_defaults_to_write_target_when_multiple_collections_are_active() {
    let (_dir, conn) = open_test_db();
    insert_collection(&conn, 2, "work", false);
    let server = QuaidServer::new(conn);
    create_page(
        &server,
        "notes/default-target",
        "---\ntitle: Default Target\ntype: note\n---\ndefault target body\n",
    );
    create_page_in_collection(
        &server,
        "work",
        "notes/work-target",
        "---\ntitle: Work Target\ntype: note\n---\nwork target body\n",
    );

    let result = server
        .memory_list(MemoryListInput {
            collection: None,
            namespace: None,
            wing: Some("notes".to_string()),
            page_type: Some("note".to_string()),
            limit: None,
        })
        .unwrap();

    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["slug"], "default::notes/default-target");
}

#[test]
fn memory_list_defaults_to_memory_collection_when_dedicated_memory_location_is_enabled() {
    let (_dir, conn) = open_test_db();
    conn.execute(
        "UPDATE config SET value = 'dedicated-collection' WHERE key = 'memory.location'",
        [],
    )
    .unwrap();
    let memory_root = turn_writer::resolve_memory_root(&conn).unwrap();
    conn.execute(
        "UPDATE collections SET needs_full_sync = 0 WHERE id = ?1",
        [memory_root.collection_id],
    )
    .unwrap();
    let server = QuaidServer::new(conn);
    server
        .memory_put(MemoryPutInput {
            slug: format!("{}::notes/memory-list", memory_root.collection_name),
            content: "---\ntitle: Memory List\ntype: note\n---\nlisted from memory collection\n"
                .to_string(),
            expected_version: None,
            namespace: Some("q0000".to_string()),
        })
        .unwrap();

    let result = server
        .memory_list(MemoryListInput {
            collection: None,
            namespace: Some("q0000".to_string()),
            wing: Some("notes".to_string()),
            page_type: Some("note".to_string()),
            limit: None,
        })
        .unwrap();

    let rows: Vec<serde_json::Value> = serde_json::from_str(&extract_text(&result)).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0]["slug"],
        format!("{}::notes/memory-list", memory_root.collection_name)
    );
}

#[test]
fn read_tools_unknown_collection_filter_errors_clearly() {
    let (_dir, conn) = open_test_db();
    let server = QuaidServer::new(conn);

    let query_error = server
        .memory_query(MemoryQueryInput {
            query: "anything".to_string(),
            collection: Some("missing".to_string()),
            namespace: None,
            wing: None,
            limit: None,
            depth: None,
            include_superseded: None,
            hops: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap_err();
    assert_eq!(query_error.code, ErrorCode(-32001));
    assert!(query_error
        .message
        .contains("collection not found: missing"));

    let search_error = server
        .memory_search(MemorySearchInput {
            query: "anything".to_string(),
            collection: Some("missing".to_string()),
            namespace: None,
            wing: None,
            limit: None,
            include_superseded: None,
            relevance_floor: None,
            max_chunks_per_doc: None,
            redact: None,
        })
        .unwrap_err();
    assert_eq!(search_error.code, ErrorCode(-32001));
    assert!(search_error
        .message
        .contains("collection not found: missing"));

    let list_error = server
        .memory_list(MemoryListInput {
            collection: Some("missing".to_string()),
            namespace: None,
            wing: None,
            page_type: None,
            limit: None,
        })
        .unwrap_err();
    assert_eq!(list_error.code, ErrorCode(-32001));
    assert!(list_error.message.contains("collection not found: missing"));
}
