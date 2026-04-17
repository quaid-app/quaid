# Import Type Inference — Implementation Checklist

**Scope:** Add folder-based type inference as tier-2 fallback in `gbrain import`;
document the type inference rules.
Closes: #40

---

## Phase A — type inference in `src/core/migrate.rs`

- [x] A.1 Add `infer_type_from_path(file_path: &Path, root: &Path) -> Option<String>` function:
  - Compute the relative path from `root`.
  - Extract the first path component (top-level folder name).
  - Strip leading `[0-9]+\.\s*` prefix (regex or manual char scan).
  - Convert to lowercase.
  - Match against the following table and return `Some(type_str)` on a hit, `None` on miss:
    - `projects` → `"project"`
    - `areas` → `"area"`
    - `resources` → `"resource"`
    - `archives` → `"archive"`
    - `journal` | `journals` → `"journal"`
    - `people` → `"person"`
    - `companies` | `orgs` → `"company"`

- [x] A.2 Modify `parse_file` to use a two-tier lookup:
  ```rust
  let (page_type, type_inferred) = if let Some(t) = frontmatter.get("type") {
      (t.clone(), false)
  } else if let Some(t) = infer_type_from_path(file_path, root) {
      (t, true)
  } else {
      ("concept".to_string(), false)
  };
  ```
  Thread `type_inferred` through `ParsedEntry` or log it locally.

- [x] A.3 Log inferred type in the import progress output when `type_inferred == true`:
  `Imported <slug> (inferred type: <type>)` — only when tier 2 fired. Use the existing
  progress printer; do not add a new logging mechanism.

---

## Phase B — documentation

- [x] B.1 Add a "Page types and PARA structure" section to `docs/getting-started.md`:
  - List the supported folder → type mappings in a table.
  - Explain that `type:` in frontmatter always overrides folder inference.
  - Note that the fallback for unrecognized folders is `concept`.
  - Show an example: importing `2. Areas/Health/exercise.md` yields type `area`.

- [x] B.2 Update `gbrain import --help` text (in `src/commands/import.rs` or equivalent) to
  note that page types are inferred from top-level folder name when not set in frontmatter.

---

## Phase C — tests

- [x] C.1 Unit tests for `infer_type_from_path`:
  - `1. Projects/foo/bar.md` → `project`
  - `2. Areas/health.md` → `area`
  - `Resources/book.md` → `resource`
  - `Journal/2024-01-01.md` → `journal`
  - `people/alice.md` → `person`
  - `random/note.md` → `None` (falls through)

- [x] C.2 Integration test: import a minimal PARA-structured directory with no `type:`
  frontmatter fields. Verify stats show correct type distribution.

- [x] C.3 Test that explicit frontmatter `type:` always wins over folder inference:
  a file at `Projects/note.md` with `type: concept` in frontmatter must be typed `concept`.

---

## Phase D — verification

- [x] D.1 Import the beta tester's vault structure (or representative subset). Run
  `gbrain stats`. Confirm `concept` count drops to near-zero for PARA-structured content
  and project/area/resource/archive counts are proportional to folder sizes.
