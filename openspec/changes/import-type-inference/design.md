## Context

`src/core/migrate.rs` `parse_file()` derives a page's `type` from frontmatter:

```rust
let page_type = frontmatter
    .get("type")
    .cloned()
    .unwrap_or_else(|| "concept".to_string());
```

When `type:` is absent from frontmatter, the type is unconditionally `"concept"`.
The relative file path (used to derive the slug) is available in `parse_file` as
`file_path` and `root`, but is currently only used for slug derivation.

PARA vault folder naming conventions: Obsidian users commonly name their top-level
folders with numeric prefixes (`1. Projects`, `2. Areas`, etc.) or without (`Projects`,
`Areas`). Both forms must be handled.

---

## Decisions

### 1. Two-tier fallback: frontmatter first, then folder inference

**Decision:** The existing frontmatter read is unchanged and remains tier 1. Folder-based
inference is tier 2, only applied when `frontmatter["type"]` is absent. The final fallback
remains `"concept"`.

**Rationale:** Frontmatter is explicit author intent and must always win. Folder inference
is a convention-based heuristic; if the author has set a `type:` field, their intent is clear.

### 2. Case-insensitive, numeric-prefix-stripped matching

**Decision:** Normalize the first path component by:
1. Stripping a leading `[0-9]+\.\s*` prefix (Obsidian PARA numbering).
2. Converting to lowercase.
3. Matching against a fixed table of known folder names.

**Rationale:** Beta tester's vault used `1. Projects`, `2. Areas` etc. Hardcoding without
normalization would require separate entries for every numeric variant. Case-insensitive
matching is table-stakes for user-facing path matching.

### 3. Supported folder → type mappings (initial set)

The initial mapping covers the PARA structure and the two most common named wings:

| Normalized folder  | Type       |
|--------------------|------------|
| `projects`         | `project`  |
| `areas`            | `area`     |
| `resources`        | `resource` |
| `archives`         | `archive`  |
| `journal`          | `journal`  |
| `journals`         | `journal`  |
| `people`           | `person`   |
| `companies`        | `company`  |
| `orgs`             | `company`  |

Anything not in the table falls through to `"concept"`.

### 4. Log inferred type only when tier 2 was used

**Decision:** The import progress output notes `(inferred type: X)` only when the type was
inferred from the folder, not when it came from frontmatter or when the fallback `"concept"`
was used unchanged.

**Rationale:** Logging every type assignment would be too verbose for large vaults. Users
need to see when the heuristic fired, not when frontmatter was honored as expected.

### 5. No config-file mapping in this lane

**Decision:** The folder-to-type table is hardcoded in this change. A user-configurable
mapping file is deferred.

**Rationale:** The PARA mapping covers the target user base. A config format adds surface
area and documentation burden. The hardcoded table can be extended in a follow-on change
without breaking any interface.
