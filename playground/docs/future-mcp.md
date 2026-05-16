# Future MCP Endpoints

The playground intentionally avoids Rust changes. These endpoints would make the
UI cleaner later, but are not required for the initial playground.

## SLM Chat / Inference

Expose the internal SLM runner through MCP for basic local assistant behavior.

Suggested tool names:

- `slm_infer`
- `memory_assistant_chat`

Suggested inputs:

- `model_alias`
- `messages` or `prompt`
- `max_tokens`
- later: `temperature`, `top_p`, streaming, and chat template selection

The playground can then retrieve context with `memory_query`, pass that context
to the local SLM, store assistant turns with `memory_add_turn`, and run fact
extraction over the conversation.

## File and Raw Import APIs

Dedicated MCP tools would avoid read-only SQLite/file inspection from the
playground backend:

- list collections and root paths
- list synced files
- read active raw import bytes
- read live collection files with root containment handled by Quaid

## Model Management APIs

Dedicated MCP tools would replace the CLI bridge for:

- model download/status
- extraction enable/disable/status
- embedding profile creation and re-embedding

## Graph APIs

A whole-knowledge-base graph endpoint would avoid the playground building a
graph from read-only SQLite rows. The current focused graph flow can use
`memory_graph`.
