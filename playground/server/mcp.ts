import { callToolViaCli } from "./cli";
import { getConfig } from "./config";
import { ApiError } from "./http";

type PendingRequest = {
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
};

type SseEvent = {
  event?: string;
  data: string;
};

class McpSseClient {
  private endpoint: string | null = null;
  private endpointPromise: Promise<string> | null = null;
  private idCounter = 1;
  private pending = new Map<number, PendingRequest>();
  private connectedUrl = "";

  async request(method: string, params?: unknown): Promise<unknown> {
    const config = getConfig();
    if (this.connectedUrl !== config.mcpUrl) {
      this.reset();
      this.connectedUrl = config.mcpUrl;
    }
    const endpoint = await this.ensureEndpoint(config.mcpUrl);
    const id = this.idCounter++;
    const responsePromise = new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      setTimeout(() => {
        if (this.pending.delete(id)) {
          reject(new ApiError(504, `MCP request timed out: ${method}`));
        }
      }, 30_000);
    });

    const postUrl = new URL(endpoint, config.mcpUrl).toString();
    const postResponse = await fetch(postUrl, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id,
        method,
        params
      })
    });
    if (!postResponse.ok) {
      this.pending.delete(id);
      throw new ApiError(postResponse.status, `MCP POST failed: ${postResponse.statusText}`);
    }

    return responsePromise;
  }

  private reset(): void {
    this.endpoint = null;
    this.endpointPromise = null;
    for (const pending of this.pending.values()) {
      pending.reject(new ApiError(503, "MCP client reset"));
    }
    this.pending.clear();
  }

  private ensureEndpoint(baseUrl: string): Promise<string> {
    if (this.endpoint) {
      return Promise.resolve(this.endpoint);
    }
    if (!this.endpointPromise) {
      this.endpointPromise = this.connect(baseUrl);
    }
    return this.endpointPromise;
  }

  private async connect(baseUrl: string): Promise<string> {
    const response = await fetch(new URL("/sse", baseUrl), {
      headers: { accept: "text/event-stream" }
    });
    const body = response.body;
    if (!response.ok || !body) {
      throw new ApiError(response.status || 502, `Failed to open MCP SSE stream: ${response.statusText}`);
    }

    const endpointPromise = new Promise<string>((resolve, reject) => {
      const timeout = setTimeout(() => reject(new ApiError(504, "MCP endpoint event timed out")), 10_000);
      this.readSse(body, (event) => {
        if (event.data.includes("/message")) {
          clearTimeout(timeout);
          this.endpoint = event.data.trim();
          resolve(this.endpoint);
          return;
        }
        this.handleMessage(event.data);
      }).catch((error) => {
        clearTimeout(timeout);
        this.reset();
        reject(error);
      });
    });

    return endpointPromise;
  }

  private async readSse(
    body: ReadableStream<Uint8Array>,
    onEvent: (event: SseEvent) => void
  ): Promise<void> {
    const reader = body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";

    while (true) {
      const { value, done } = await reader.read();
      if (done) {
        break;
      }
      buffer += decoder.decode(value, { stream: true }).replace(/\r\n/g, "\n");
      let boundary = buffer.indexOf("\n\n");
      while (boundary >= 0) {
        const block = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        const event = parseSseEvent(block);
        if (event.data) {
          onEvent(event);
        }
        boundary = buffer.indexOf("\n\n");
      }
    }
  }

  private handleMessage(data: string): void {
    let message: { id?: number; result?: unknown; error?: unknown };
    try {
      message = JSON.parse(data) as { id?: number; result?: unknown; error?: unknown };
    } catch {
      return;
    }
    if (typeof message.id !== "number") {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    if (message.error) {
      pending.reject(new ApiError(502, "MCP tool returned an error", message.error));
    } else {
      pending.resolve(message.result);
    }
  }
}

function parseSseEvent(block: string): SseEvent {
  const event: SseEvent = { data: "" };
  for (const line of block.split(/\r?\n/)) {
    if (line.startsWith("event:")) {
      event.event = line.slice("event:".length).trim();
    } else if (line.startsWith("data:")) {
      event.data += `${line.slice("data:".length).trim()}\n`;
    }
  }
  event.data = event.data.trim();
  return event;
}

const client = new McpSseClient();

function extractCallResult(result: unknown): unknown {
  if (!result || typeof result !== "object") {
    return result;
  }
  const maybeContent = (result as { content?: unknown }).content;
  if (!Array.isArray(maybeContent)) {
    return result;
  }
  const text = maybeContent
    .map((item) => {
      if (item && typeof item === "object") {
        const rawText = (item as { text?: unknown }).text;
        if (typeof rawText === "string") {
          return rawText;
        }
      }
      return "";
    })
    .join("")
    .trim();
  if (!text) {
    return result;
  }
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

export async function listMcpTools(): Promise<unknown> {
  try {
    return await client.request("tools/list");
  } catch (error) {
    const config = getConfig();
    if (!config.cliFallbackForMcp) {
      throw error;
    }
    return {
      fallback: "cli",
      tools: [
        "memory_get",
        "memory_put",
        "memory_list",
        "memory_raw",
        "memory_query",
        "memory_search",
        "memory_link",
        "memory_link_close",
        "memory_backlinks",
        "memory_graph",
        "memory_check",
        "memory_timeline",
        "memory_tags",
        "memory_gap",
        "memory_gaps",
        "memory_add_turn",
        "memory_close_session",
        "memory_close_action",
        "memory_correct",
        "memory_correct_continue",
        "memory_stats",
        "memory_collections",
        "memory_namespace_create",
        "memory_namespace_destroy"
      ],
      reason: error instanceof Error ? error.message : String(error)
    };
  }
}

export async function callMcpTool(name: string, args: unknown): Promise<unknown> {
  try {
    const result = await client.request("tools/call", {
      name,
      arguments: args ?? {}
    });
    return extractCallResult(result);
  } catch (error) {
    const config = getConfig();
    if (!config.cliFallbackForMcp) {
      throw error;
    }
    return callToolViaCli(name, args ?? {});
  }
}
