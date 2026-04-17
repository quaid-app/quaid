## Brain Config Schema Spec

### Table DDL

```sql
CREATE TABLE IF NOT EXISTS brain_config (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) STRICT;
```

### Required Keys (written at `gbrain init`)

| Key | Example Value | Description |
|-----|--------------|-------------|
| `model_id` | `BAAI/bge-small-en-v1.5` | Full HuggingFace model ID |
| `model_alias` | `small` | Shorthand alias, or `custom` if a full ID was provided |
| `embedding_dim` | `384` | Integer embedding dimension for this model |
| `schema_version` | `4` | Current schema version for migration tracking |

### Init Behaviour

- `gbrain init` writes all four keys on first creation.
- If the DB already exists and `brain_config` is populated, `gbrain init` is a no-op (idempotent).
- If the DB exists but `brain_config` is missing (pre-v0.9.2 DB), `gbrain init` writes the config keys using the default model (`small`), emitting a migration notice.
