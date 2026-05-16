# Quaid Playground

React/Vite playground for testing Quaid memory, conversations, extraction,
models, files, and graph views.

## WSL Startup

Run from WSL because Quaid itself is Linux-only in this setup.

```bash
cd /mnt/d/repos/quaid
cargo build

cd /mnt/d/repos/quaid/playground
pnpm install

QUAID_BIN=/mnt/d/repos/quaid/target/debug/quaid \
QUAID_DB="$HOME/.quaid/memory.db" \
pnpm dev --host 127.0.0.1
```

Managed mode auto-starts:

```bash
quaid --db "$QUAID_DB" --model small serve --http --port 3112 --trust-loopback
```

If the DB is missing, the playground creates the parent directory and runs:

```bash
quaid --model small init "$QUAID_DB"
```

Runtime logs appear in two places:

- the Vite terminal as `[quaid-runtime] ...`
- the left status panel under **Runtime log**

## Useful Environment Variables

- `QUAID_BIN`: path to the Linux Quaid binary.
- `QUAID_DB`: DB path, defaults to `~/.quaid/memory.db`.
- `QUAID_MODEL`: embedding model alias, defaults to `small`.
- `QUAID_MCP_URL`: external MCP URL; when set, the playground does not manage `quaid serve`.
- `QUAID_PLAYGROUND_AUTO_INIT`: auto-init missing DBs, defaults to `true`.
- `QUAID_PLAYGROUND_AUTO_ENABLE_EXTRACTION`: auto-run `quaid extraction enable`, defaults to `false` because it may download an SLM model.

To auto-enable fact extraction at startup:

```bash
QUAID_PLAYGROUND_AUTO_ENABLE_EXTRACTION=1 pnpm dev --host 127.0.0.1
```
