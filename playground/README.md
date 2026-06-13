# Quaid Playground

React/Vite playground for testing Quaid memory, conversations, extraction,
models, files, and graph views.

## Quick start (macOS / Linux)

The playground runs natively on macOS and Linux against a locally built Quaid
binary. By default it uses a **throwaway in-repo database** at
`playground/.data/memory.db` so it never touches your real `~/.quaid/memory.db`.

```bash
# From the repository root: build the binary the playground will drive.
cargo build

# Then start the playground.
cd playground
pnpm install
pnpm serve
```

`pnpm serve` points `QUAID_BIN` at `../target/release/quaid` and `QUAID_DB` at
the in-repo `.data/memory.db`. If you built a debug binary instead, run Vite
directly and override `QUAID_BIN`:

```bash
QUAID_BIN=../target/debug/quaid pnpm dev --host 127.0.0.1
```

To point at your real memory instead of the throwaway DB, set `QUAID_DB`
explicitly:

```bash
QUAID_DB="$HOME/.quaid/memory.db" pnpm dev --host 127.0.0.1
```

Managed mode auto-starts the HTTP transport:

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

- `QUAID_BIN`: path to the Quaid binary (defaults to `quaid` on `PATH`).
- `QUAID_DB`: DB path, defaults to the in-repo `playground/.data/memory.db`.
- `QUAID_MODEL`: embedding model alias, defaults to `small`.
- `QUAID_MCP_URL`: external MCP URL; when set, the playground does not manage `quaid serve`.
- `QUAID_PLAYGROUND_AUTO_INIT`: auto-init missing DBs, defaults to `true`.
- `QUAID_PLAYGROUND_AUTO_ENABLE_EXTRACTION`: auto-run `quaid extraction enable`, defaults to `false` because it may download an SLM model.

To auto-enable fact extraction at startup:

```bash
QUAID_PLAYGROUND_AUTO_ENABLE_EXTRACTION=1 pnpm dev --host 127.0.0.1
```
