use quaid::commands::put;
use quaid::core::db;
use quaid::core::search::hybrid_search_canonical_with_namespace;

#[test]
fn namespaced_write_is_visible_to_namespace_global_filter_and_unfiltered_query() {
    let conn = db::open(":memory:").expect("open db");
    let content =
        "---\ntitle: Namespace Probe\ntype: concept\n---\nnamespaceprobe unique evidence\n";

    put::put_from_string_with_namespace(
        &conn,
        "notes/namespace-probe",
        content,
        Some("test-ns"),
        None,
    )
    .expect("write namespaced page");

    let namespaced = hybrid_search_canonical_with_namespace(
        "namespaceprobe",
        None,
        None,
        Some("test-ns"),
        &conn,
        10,
    )
    .expect("query namespace");
    let global_only =
        hybrid_search_canonical_with_namespace("namespaceprobe", None, None, Some(""), &conn, 10)
            .expect("query global namespace");
    let unfiltered =
        hybrid_search_canonical_with_namespace("namespaceprobe", None, None, None, &conn, 10)
            .expect("query all namespaces");

    assert!(namespaced
        .iter()
        .any(|result| result.slug == "default::notes/namespace-probe"));
    assert!(global_only.is_empty());
    assert!(unfiltered
        .iter()
        .any(|result| result.slug == "default::notes/namespace-probe"));
}
