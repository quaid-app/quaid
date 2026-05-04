# Skill: OpenSpec Scope Mismatch Fix

**Context:** When `tasks.md` and `implementation_plan.md` disagree on what a task does within a batch, resolve the mismatch in the artifacts before any code starts.

---

## When to use

- `tasks.md` describes a task with a broader/different scope than `implementation_plan.md` specifies for the current batch
- A security-critical subsystem is described in the client code before the server design is locked
- The implementation plan defers something to a later batch that `tasks.md` still presents as in-scope

---

## Pattern

### 1. Identify the authoritative source

`implementation_plan.md` is the batch-level authority. It describes WHAT ships in WHICH batch.
`tasks.md` is the task-level authority for ACCEPTANCE CRITERIA once batch scope is settled.

When they conflict: fix `tasks.md` to match `implementation_plan.md` for the current batch. Do not expand batch scope by leaving the old tasks.md description in place.

### 2. Minimal fix — rewrite only the conflicting task line

Replace only the conflicting task description. Keep the task ID, checkbox state, and placement identical.

**Pattern for narrowing a task:**
```markdown
- [ ] 12.6a <narrowed description matching implementation_plan batch scope>. **Batch N scope:** <what this batch actually delivers>; <deferred work> is deferred to Batch M (tasks X–Y).
  > **Scope note (Author, DATETIME):** Original description specified <original scope>. Batch N narrows this to <narrowed shape> because <reason>. Batch M tasks will complete the original scope.
```

Key elements:
- State the batch scope limitation explicitly inline
- Name the future batch and task IDs that complete the original scope
- Add a `> **Scope note**` annotation with author + datetime for audit trail
- Do NOT delete or rephrase the future-batch tasks (e.g., 12.6c–g) — they are the authoritative spec for that batch

### 2d. Add/add conflicts against main resolve to shipped truth

If main and your branch both add the same OpenSpec change files, do not treat the merge as a coin flip.

- Compare both versions against the shipped code and the current completed-task state.
- Keep the variant whose proposal/design/specs/tasks describe the landed baseline and any truthful scope notes.
- Reject older draft wording that still talks about future schema bumps, unchecked tasks, or broader surfaces that never shipped.

When the branch copy is the truthful one, resolve the conflict to that content and continue the merge. The goal is a mergeable PR with artifacts that remain auditable after landing, not a textual blend of two incompatible histories.
### 3. Update stale completion counts

If `implementation_plan.md` has a stale "current completion state" section, update the counts to match what `openspec instructions apply` reports. This prevents confusion about remaining work.

### 4. Commit with a clear rationale

Commit message must explain:
- What the mismatch was
- What the fix is
- What is explicitly preserved / unchanged
- What batch/tasks own the deferred work

---

## Anti-patterns

- ❌ Implementing the broader tasks.md scope because it was there first
- ❌ Deleting future-batch task descriptions to "remove confusion" — those are the spec for the next batch
- ❌ Splitting a narrowed task into a new task ID — rewrite the existing task line in place
- ❌ Making the fix in the main clone checkout if a worktree was designated for this batch

---

## Security-surface isolation rule

When the mismatch involves a security-critical subsystem (e.g., IPC socket authentication, kernel peer-UID verification), the isolation is mandatory, not optional:

> The server-side design must land and be reviewed as its own batch before client code is built against it.

Symptoms of violation:
- Client code references error types or socket paths before the server task exists
- Tests mock an IPC socket that has no implementation
- A "for now" comment in the implementation plan explains why the task is a stub

In these cases: the stub is the correct Batch N scope. The full implementation is the correct Batch N+1 scope.

---

## Examples

**Vault-sync-engine Batch 4:**
- Mismatch: `tasks.md 12.6a` said "Proxy mode over IPC" but `implementation_plan.md` Batch 4 said "refuse-when-live stub"
- Fix: Rewrote `tasks.md 12.6a` to refuse-when-live shape; added scope note pointing to Batch 5 for IPC proxy upgrade
- IPC tasks `12.6c–g` in `tasks.md` left untouched — they are Batch 5's authoritative spec

