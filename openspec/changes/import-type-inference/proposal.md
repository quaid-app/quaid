---
id: import-type-inference
title: "import: respect frontmatter type and infer from PARA folder structure"
status: proposed
type: enhancement
owner: fry
reviewers: [leela]
created: 2026-04-19
closes: ["#40"]
---

# import: respect frontmatter type and infer from PARA folder structure

## Why

`gbrain import` assigns every page the type `concept` unless the page's frontmatter contains
an explicit `type:` field. For the dominant Obsidian PKM structure — PARA (Projects, Areas,
Resources, Archives) — this means ~99% of pages land as `concept`, making type-filtered
queries useless and the stats output misleading.

Beta tester doug-aillm (issue #40) reproduced this: after importing a 789-page PARA vault,
stats showed 784/789 pages as `concept`. The target user base is the Obsidian/PARA community;
ignoring folder structure is a first-run blocker for this cohort.

The code already reads the frontmatter `type:` field. The gap is the fallback: when `type:` is
absent, the code hard-codes `"concept"` without consulting the file's path.

## What Changes

### 1. `src/core/migrate.rs` — tiered type inference in `parse_file`

Replace the single-fallback `"concept"` default with a three-tier fallback:

**Tier 1 — frontmatter field (existing, unchanged):**
If `frontmatter["type"]` is present and non-blank/non-null, use it verbatim.

**Tier 2 — top-level folder inference (new):**
If `frontmatter["type"]` is absent or blank, inspect the first path component of the relative file
path (the top-level folder) and map it to a type:

| Folder (case-insensitive exact match after prefix strip) | Inferred type |
|----------------------------------------|---------------|
| `projects` / `1. projects`             | `project`     |
| `areas`    / `2. areas`                | `area`        |
| `resources`/ `3. resources`            | `resource`    |
| `archives` / `4. archives`             | `archive`     |
| `journal`  / `journals`                | `journal`     |
| `people`                               | `person`      |
| `companies`/ `orgs`                    | `company`     |
| anything else                          | `concept`     |

The matching is case-insensitive and strips leading numeric prefixes (`1. `, `2. `, etc.) to
handle Obsidian PARA numbered folder naming conventions.

**Tier 3 — final fallback (unchanged):**
`"concept"` if no folder mapping matches.

### 2. `src/core/migrate.rs` — expose inferred type in import output

The import command's progress output already logs the slug. Extend it to log the inferred type
when it differs from the frontmatter value (i.e., when tier 2 was used), so users can verify
the inference is correct:

```
Imported people/alice (inferred type: person)
```

### 3. Docs — document type inference rules

- `docs/getting-started.md`: add a "Page types and PARA structure" section documenting the
  folder-to-type mapping and how to override it with a frontmatter `type:` field.
- `gbrain import --help` text: add a note that types are inferred from folder structure when
  not set in frontmatter.

## Non-Goals

- Inferring types from page content or body text — folder structure and frontmatter only.
- Supporting arbitrary custom folder-to-type mappings via config file — deferred; the built-in
  PARA mapping covers the majority of users.
- Changing how types are stored in the database — the `type` column is unchanged.
- Retroactively re-typing already-imported pages — users who want updated types must
  re-import or use `gbrain put` to update individual pages.

## Impact

- `src/core/migrate.rs`: new `infer_type_from_path` function; modified `parse_file` to call it
  as fallback when frontmatter `type:` is absent.
- `docs/getting-started.md`: new "Page types and PARA structure" section.
- Import behavior change: PARA-structured vaults will now yield meaningful type distributions
  on first import without any frontmatter changes.
