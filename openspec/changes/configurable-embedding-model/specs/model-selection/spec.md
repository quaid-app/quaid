## Model Selection Spec

### Interface

**Environment variable (primary):**
```
QUAID_MODEL=large quaid query "stablecoin regulation"
QUAID_MODEL=BAAI/bge-large-en-v1.5 quaid query "stablecoin regulation"
```

**CLI flag (alternative):**
```
quaid --model large query "stablecoin regulation"
quaid --model BAAI/bge-m3 query "stablecoin regulation"
```

**Precedence:** `--model` flag > `QUAID_MODEL` env var > default (`small`)

### Alias Resolution

| Alias | HuggingFace Model ID | Dimensions | Notes |
|-------|---------------------|-----------|-------|
| `small` | BAAI/bge-small-en-v1.5 | 384 | Default, unchanged behaviour |
| `base` | BAAI/bge-base-en-v1.5 | 768 | |
| `large` | BAAI/bge-large-en-v1.5 | 1024 | |
| `m3` | BAAI/bge-m3 | 1024 | Multilingual |
| Any other string | Used as-is as HuggingFace model ID | From model config | Warning: no SHA-256 pin |

### Airgapped Channel Behaviour

The `embedded-model` feature (airgapped build) always uses the embedded BGE-small weights. If `QUAID_MODEL` or `--model` is set to anything other than `small` or `BAAI/bge-small-en-v1.5`, emit a clear warning and continue with BGE-small. Do not error out.

### SHA-256 Integrity Verification

Standard aliases (`small`, `base`, `large`, `m3`) must have pinned SHA-256 hashes for `config.json`, `tokenizer.json`, and `model.safetensors`. Custom model IDs skip hash verification with a logged warning.
