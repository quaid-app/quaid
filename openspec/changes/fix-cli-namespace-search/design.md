## Context

The existing `namespace-isolation` OpenSpec added namespace filtering across read paths and
states that omitted CLI/MCP namespace reads default to global-only. GitHub issue #145 shows
that `quaid search --namespace workns "bitcoin"` can still leak results from unrelated
namespaces.

`quaid search` is an FTS5-only command. It does not initialize embeddings and should not
depend on the airgapped vs online model channel. Both release channels compile the same
`src/main.rs`, `src/commands/search.rs`, and `src/core/fts.rs` search filtering path.

## Call Chain

### CLI dispatch

`src/main.rs` defines:

```rust
Commands::Search {
    query,
    wing,
    namespace,
    limit,
    raw,
}
```

The dispatch must pass a concrete read scope into the command layer:

```rust
commands::search::run(
    &db,
    &query,
    wing,
    namespace.as_deref().or(Some("")),
    limit,
    cli.json,
    raw,
)
```

This means:

- omitted `--namespace` becomes `Some("")`
- `--namespace workns` becomes `Some("workns")`
- the CLI does not pass `None` for read searches

### Command layer

`src/commands/search.rs::run()` must validate and preserve that namespace:

```rust
crate::core::namespace::validate_optional_namespace(namespace)?;
let namespace = namespace.or(Some(""));
```

Then it must call namespace-aware helpers in both modes:

```rust
if raw {
    search_fts_canonical_with_namespace(..., namespace, db, ...)
} else {
    search_fts_canonical_tiered_with_namespace(..., namespace, db, ...)
}
```

The bug exists if either branch calls a non-namespace helper or passes `None` after the CLI
provided `Some("workns")`.

### Core FTS helper

`src/core/fts.rs::search_fts_canonical_with_namespace()` delegates to
`search_fts_internal(..., canonical_slug = true)`. The SQL builder must apply these exact
semantics:

```rust
if let Some(namespace) = namespace_filter {
    if namespace.is_empty() {
        sql.push_str(" AND p.namespace = ?");
        params.push(Box::new(String::new()));
    } else {
        sql.push_str(" AND (p.namespace = ?");
        sql.push_str(&(params.len() + 1).to_string());
        sql.push_str(" OR p.namespace = '')");
        params.push(Box::new(namespace.to_owned()));
    }
}
```

That produces the effective predicate:

- global-only: `p.namespace = ''`
- explicit namespace: `(p.namespace = 'workns' OR p.namespace = '')`
- all namespaces: no predicate, only when an internal caller deliberately passes `None`

The implementation should keep the parameterized SQL shape rather than interpolating the
namespace value into SQL text.

## Test Design

Add one subprocess integration test using the real `quaid` binary:

1. Create a temp database.
2. Insert three FTS-visible pages containing the same unique term, for example `bitcoin`:
   - `namespace = ''`, slug `notes/global-bitcoin`
   - `namespace = 'workns'`, slug `notes/work-bitcoin`
   - `namespace = 'otherns'`, slug `notes/other-bitcoin`
3. Run `quaid --db <db> --json search --namespace workns bitcoin`.
4. Assert stdout is valid JSON.
5. Assert result slugs include the global and `workns` pages.
6. Assert result slugs do not include the `otherns` page.
7. Run `quaid --db <db> --json search bitcoin`.
8. Assert omitted namespace returns only the global page.

This test catches the exact issue #145 CLI surface, including `src/main.rs` dispatch,
`src/commands/search.rs` normalization, query sanitization/tiered FTS fallback, and the
core FTS SQL predicate.

## Airgapped vs Online

There should be no behavioral difference between airgapped and online binaries for this
bug. `quaid search` uses FTS5 helpers and does not call embedding inference. The
`embedded-model` and `online-model` feature gates live in `src/core/inference.rs`; they do
not conditionalize `src/commands/search.rs` or `src/core/fts.rs`.

Validation should include the default feature set at minimum. If CI has an online-channel
job, the same CLI integration test should run there as part of the normal test matrix, but
the code fix itself is shared.

## Non-Goals

- No change to MCP `memory_search` unless the same regression is found there.
- No change to `quaid query`; it uses hybrid search and already carries namespace through
  `hybrid_search_canonical_with_namespace`.
- No schema migration.
- No new all-namespace CLI mode.
