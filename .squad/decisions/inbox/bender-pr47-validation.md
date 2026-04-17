# Bender Decision: PR #47 Validation Plan and Blocker Status

**Date:** 2026-04-19  
**Author:** Bender  
**Context:** PR #47 validation against Professor and Nibbler review blocking findings

## Verdict

**BLOCKED — Three blockers remain unfixed**

PR #47 (feat: configurable embedding model) has meaningful progress on many review comments, but the three high-severity concurrency/integrity blockers identified by Professor and Nibbler are **NOT YET ADDRESSED** in the current branch state (commit `96807dd`).

## Blocker Status

### Blocker 1: Active-model registry transition is non-atomic ❌

**Owners:** Professor (finding #2), Nibbler (finding #1)  
**File:** `src/core/db.rs:182-207` (`ensure_embedding_model_registry`)  
**Issue:** The function executes two separate autocommit statements:
1. `UPDATE embedding_models SET active = 0 WHERE active != 0` (line 188-191)
2. `INSERT ... ON CONFLICT DO UPDATE SET active = 1` (line 192-204)

**Risk:** Any concurrent reader between these two statements observes zero active models and fails with "no active embedding model configured". A crash between the two statements leaves the DB permanently broken.

**Required fix:** Wrap both statements in a single transaction (same pattern as `write_brain_config` at lines 225-252).

**Evidence of non-fix:** Examined `ensure_embedding_model_registry` — no `unchecked_transaction()` wrapper present. `write_brain_config` was correctly fixed with a transaction, but the registry flip was not.

---

### Blocker 2: Shared temp-file race on concurrent cold-start downloads ❌

**Owner:** Nibbler (finding #2)  
**File:** `src/core/inference.rs:659-702` (`download_model_file`)  
**Issue:** Downloads use fixed temp file names (`config.json.download`, `tokenizer.json.download`, `model.safetensors.download`) inside the shared cache directory. Two concurrent processes/threads downloading the same model can clobber each other's temp files, causing rename failures or cache corruption.

**Required fix:** Either:
- Use unique temp file names (e.g., append thread ID or random suffix), OR
- Add a per-model download lock (e.g., flock on cache directory or atomic marker file)

**Evidence of non-fix:** Line 667 still has `let temp_destination = cache_dir.join(format!("{file_name}.download"));` — fixed name, not unique per caller.

---

### Blocker 3: Online-model CI tests are not hermetic ❌

**Owner:** Nibbler (finding #3)  
**File:** `.github/workflows/ci.yml:70-71`  
**Issue:** The online-model test job runs `cargo test --verbose --no-default-features --features bundled,online-model` with no `GBRAIN_FORCE_HASH_SHIM=1` environment variable. This allows tests to attempt real Hugging Face downloads with 300s timeouts, making CI flaky/slow and dependent on external network health.

**Required fix:** Either:
- Set `GBRAIN_FORCE_HASH_SHIM=1` in the CI environment for the online-model test job, OR
- Split networked integration tests out of required CI gate

**Evidence of non-fix:** `.github/workflows/ci.yml:71` shows no `env:` block setting `GBRAIN_FORCE_HASH_SHIM`. The `inference.rs:1233` test does set the env var locally via `EnvVarGuard`, but that's per-test, not CI-global.

---

## Validation Plan (for post-fix execution)

Once Fry addresses the three blockers above, the following checks will prove the fixes:

### 1. Atomic active-model transition

**Commands:**
```bash
cargo test --test model_registry_atomic
cargo test db::tests::ensure_embedding_model_registry
```

**Manual check:** Verify `src/core/db.rs` `ensure_embedding_model_registry` wraps both statements in a transaction:
```rust
let tx = conn.unchecked_transaction()?;
tx.execute("UPDATE embedding_models SET active = 0 WHERE active != 0", [])?;
tx.execute("INSERT ... ON CONFLICT DO UPDATE SET active = 1", params![...])?;
tx.commit()?;
```

**Proof:** No gap state observable; crash recovery leaves exactly one active model.

---

### 2. Safe concurrent cache publish

**Commands:**
```bash
# Test concurrent first-use loads (if test added):
cargo test --test concurrent_download_safety --features online-model

# OR manual check of temp-file uniqueness in download_model_file
grep -A5 "temp_destination" src/core/inference.rs
```

**Required pattern (one of):**
- Unique temp file: `format!("{file_name}.download.{}", uuid/thread_id/random)`
- OR per-model lock before download loop

**Proof:** Two threads can download the same model concurrently without rename collisions.

---

### 3. Hermetic online-model CI/test behavior

**Command:**
```bash
# Verify CI sets GBRAIN_FORCE_HASH_SHIM for online-model job:
cat .github/workflows/ci.yml | grep -A10 "online channel"
```

**Required pattern:**
```yaml
- name: Run tests (online channel)
  env:
    GBRAIN_FORCE_HASH_SHIM: "1"
  run: cargo test --verbose --no-default-features --features bundled,online-model
```

**Proof:** CI online-model job completes in <60s with no network calls, regardless of Hugging Face availability.

---

## Additional validation (non-blocking, post-merge)

These checks confirm the overall model-selection contract but are NOT blockers if the three above are fixed:

1. **Model alias resolution:**
   ```bash
   cargo test resolve_model
   ```

2. **Mismatch detection:**
   ```bash
   cargo test --test model_mismatch
   ```

3. **Airgapped fallback:**
   ```bash
   cargo build --release  # default = embedded-model
   GBRAIN_MODEL=large target/release/gbrain init test.db 2>&1 | grep -i warning
   ```

4. **Online download path (requires network):**
   ```bash
   cargo build --release --no-default-features --features bundled,online-model
   GBRAIN_MODEL=base target/release/gbrain init test-online.db
   # Verify ~/.gbrain/models/BAAI_bge-base-en-v1.5/ created with 3 files + SHA-256 check
   ```

5. **Full test suite:**
   ```bash
   cargo fmt --all --check
   cargo clippy --all-targets --all-features -- -D warnings
   cargo test --verbose
   cargo test --verbose --no-default-features --features bundled,online-model
   ```

---

## Recommendation to Fry

Apply fixes for the three blockers in this order:

1. **Atomic registry flip** (easiest) — wrap `ensure_embedding_model_registry` statements in a transaction
2. **Hermetic CI** (low-risk) — add `GBRAIN_FORCE_HASH_SHIM=1` to online-model test job env
3. **Concurrent download safety** (most complex) — add per-model lock or unique temp file names

After each fix, re-run relevant test to confirm. Once all three close, ping Bender for full validation lane execution.

---

## Ownership

- **Fry:** Implementation fixes for all three blockers
- **Bender:** Validation execution once fixes land
- **Leela:** Final merge decision after validation clearance
