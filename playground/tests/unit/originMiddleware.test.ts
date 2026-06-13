import { Readable } from "node:stream";
import type { IncomingMessage, ServerResponse } from "node:http";
import { beforeAll, describe, expect, it, vi } from "vitest";

// Keep the runtime "external" so constructing the middleware never spawns the
// quaid binary, and point the DB at a path that does not exist so any read
// route degrades gracefully. Must be set before importing server modules.
beforeAll(() => {
  process.env.QUAID_MCP_URL = "http://127.0.0.1:65535";
  process.env.QUAID_DB = "/nonexistent/quaid-playground-test/memory.db";
});

// The 200 POST path must not reach a live MCP runtime; stub the MCP module so
// `/mcp/call` returns deterministically once the guards have passed.
vi.mock("../../server/mcp", () => ({
  listMcpTools: vi.fn(async () => []),
  callMcpTool: vi.fn(async (name: string) => ({ ok: true, name }))
}));

const { createPlaygroundApiMiddleware } = await import("../../server/index");

interface CapturedResponse {
  res: ServerResponse;
  done: Promise<{ status: number; body: string; contentType: string }>;
}

function makeRequest(options: {
  method: string;
  url: string;
  headers?: Record<string, string>;
  body?: string;
}): IncomingMessage {
  const stream = Readable.from(
    options.body != null ? [Buffer.from(options.body, "utf8")] : []
  ) as unknown as IncomingMessage;
  stream.method = options.method;
  stream.url = options.url;
  stream.headers = options.headers ?? {};
  return stream;
}

function makeResponse(): CapturedResponse {
  const headers: Record<string, string> = {};
  let resolveDone!: (value: { status: number; body: string; contentType: string }) => void;
  const done = new Promise<{ status: number; body: string; contentType: string }>((resolve) => {
    resolveDone = resolve;
  });
  const res = {
    statusCode: 200,
    writableEnded: false,
    setHeader(name: string, value: string) {
      headers[name.toLowerCase()] = value;
    },
    getHeader(name: string) {
      return headers[name.toLowerCase()];
    },
    end(body?: string) {
      this.writableEnded = true;
      resolveDone({
        status: this.statusCode,
        body: body ?? "",
        contentType: headers["content-type"] ?? ""
      });
    }
  } as unknown as ServerResponse;
  return { res, done };
}

async function invoke(req: IncomingMessage): Promise<{ status: number; body: string; contentType: string }> {
  const middleware = createPlaygroundApiMiddleware();
  const { res, done } = makeResponse();
  await new Promise<void>((resolve, reject) => {
    middleware(req, res, (error?: unknown) => {
      if (error) {
        reject(error);
      } else {
        resolve();
      }
    });
    // The middleware resolves the response asynchronously; surface completion.
    done.then(() => resolve()).catch(reject);
  });
  return done;
}

describe("playground API origin/content-type guard (integration)", () => {
  it("returns 403 for a foreign Origin", async () => {
    const result = await invoke(
      makeRequest({
        method: "POST",
        url: "/mcp/call",
        headers: {
          host: "127.0.0.1:5174",
          origin: "https://evil.example.com",
          "content-type": "application/json"
        },
        body: JSON.stringify({ name: "memory_stats" })
      })
    );
    expect(result.status).toBe(403);
    expect(result.body).toContain("Forbidden origin");
  });

  it("returns 403 for a non-loopback Host (DNS rebinding)", async () => {
    const result = await invoke(
      makeRequest({
        method: "GET",
        url: "/status",
        headers: { host: "evil.example.com" }
      })
    );
    expect(result.status).toBe(403);
    expect(result.body).toContain("Forbidden host");
  });

  it("returns 415 for a same-origin text/plain body", async () => {
    const result = await invoke(
      makeRequest({
        method: "POST",
        url: "/mcp/call",
        headers: {
          host: "127.0.0.1:5174",
          origin: "http://127.0.0.1:5174",
          "content-type": "text/plain"
        },
        body: JSON.stringify({ name: "memory_stats" })
      })
    );
    expect(result.status).toBe(415);
  });

  it("returns 200 for a same-origin application/json POST", async () => {
    const result = await invoke(
      makeRequest({
        method: "POST",
        url: "/mcp/call",
        headers: {
          host: "127.0.0.1:5174",
          origin: "http://127.0.0.1:5174",
          "content-type": "application/json"
        },
        body: JSON.stringify({ name: "memory_stats" })
      })
    );
    expect(result.status).toBe(200);
    expect(JSON.parse(result.body)).toMatchObject({ ok: true, name: "memory_stats" });
  });

  it("returns 200 for a same-origin GET with no cross-origin Origin", async () => {
    const result = await invoke(
      makeRequest({
        method: "GET",
        url: "/status",
        headers: { host: "127.0.0.1:5174" }
      })
    );
    expect(result.status).toBe(200);
  });
});
