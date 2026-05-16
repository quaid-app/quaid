import type { IncomingMessage, ServerResponse } from "node:http";
import { CLI_COMMANDS, runCliCommand } from "./cli";
import { publicConfig } from "./config";
import { getCollections, getDbStatus, getDbTree, getGraph, getPageBySlug, getRawImport } from "./db";
import { listLiveFiles, readLiveFile } from "./files";
import { ApiError, parseUrl, readJsonBody, requireMethod, sendError, sendJson, type Middleware } from "./http";
import { callMcpTool, listMcpTools } from "./mcp";
import { createProfile, embedProfile, listProfiles, queryProfile } from "./profiles";
import { ensureManagedRuntimeStarted, restartRuntime, runtimeStatus, startRuntime, stopRuntime } from "./runtime";
import type { CliRunRequest } from "./types";

type Handler = (req: IncomingMessage, res: ServerResponse, url: URL) => Promise<void>;

function route(pathname: string, method: string, expectedPath: string, expectedMethod: string): boolean {
  return pathname === expectedPath && method === expectedMethod;
}

async function handleRequest(req: IncomingMessage, res: ServerResponse): Promise<void> {
  const url = parseUrl(req);
  const method = req.method ?? "GET";

  const routes: Handler[] = [
    statusRoute,
    runtimeRoute,
    mcpRoute,
    cliRoute,
    dbRoute,
    filesRoute,
    profilesRoute
  ];

  for (const handler of routes) {
    const handled = await maybeHandle(handler, req, res, url);
    if (handled) {
      return;
    }
  }

  throw new ApiError(404, `Unknown API route: ${method} ${url.pathname}`);
}

async function maybeHandle(
  handler: Handler,
  req: IncomingMessage,
  res: ServerResponse,
  url: URL
): Promise<boolean> {
  const before = res.writableEnded;
  await handler(req, res, url);
  return !before && res.writableEnded;
}

async function statusRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (!route(url.pathname, req.method ?? "GET", "/status", "GET")) {
    return;
  }
  sendJson(res, 200, {
    config: publicConfig(),
    runtime: runtimeStatus(),
    database: getDbStatus()
  });
}

async function runtimeRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (!url.pathname.startsWith("/runtime/")) {
    return;
  }
  requireMethod(req, "POST");
  switch (url.pathname) {
    case "/runtime/start":
      sendJson(res, 200, startRuntime());
      return;
    case "/runtime/stop":
      sendJson(res, 200, stopRuntime());
      return;
    case "/runtime/restart":
      sendJson(res, 200, restartRuntime());
      return;
    default:
      throw new ApiError(404, `Unknown runtime route: ${url.pathname}`);
  }
}

async function mcpRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (route(url.pathname, req.method ?? "GET", "/mcp/tools", "GET")) {
    sendJson(res, 200, await listMcpTools());
    return;
  }
  if (route(url.pathname, req.method ?? "POST", "/mcp/call", "POST")) {
    const body = await readJsonBody<{ name?: string; args?: unknown }>(req);
    if (!body.name) {
      throw new ApiError(400, "MCP tool name is required");
    }
    sendJson(res, 200, await callMcpTool(body.name, body.args ?? {}));
  }
}

async function cliRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (route(url.pathname, req.method ?? "GET", "/cli/catalog", "GET")) {
    sendJson(res, 200, CLI_COMMANDS);
    return;
  }
  if (route(url.pathname, req.method ?? "POST", "/cli/run", "POST")) {
    const body = await readJsonBody<CliRunRequest>(req);
    sendJson(res, 200, await runCliCommand(body));
  }
}

async function dbRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (!url.pathname.startsWith("/db/")) {
    return;
  }
  requireMethod(req, "GET");
  switch (url.pathname) {
    case "/db/status":
      sendJson(res, 200, getDbStatus());
      return;
    case "/db/collections":
      sendJson(res, 200, getCollections());
      return;
    case "/db/tree":
      sendJson(res, 200, getDbTree());
      return;
    case "/db/page": {
      const slug = url.searchParams.get("slug");
      if (!slug) {
        throw new ApiError(400, "slug query parameter is required");
      }
      sendJson(res, 200, getPageBySlug(slug));
      return;
    }
    case "/db/raw-file": {
      const id = Number(url.searchParams.get("id"));
      if (!Number.isFinite(id)) {
        throw new ApiError(400, "numeric id query parameter is required");
      }
      sendJson(res, 200, getRawImport(id));
      return;
    }
    case "/db/graph": {
      const scope = url.searchParams.get("scope") === "focused" ? "focused" : "whole";
      const slug = url.searchParams.get("slug") ?? undefined;
      const depth = Number(url.searchParams.get("depth") ?? "2");
      sendJson(res, 200, getGraph(scope, slug, depth));
      return;
    }
    default:
      throw new ApiError(404, `Unknown DB route: ${url.pathname}`);
  }
}

async function filesRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (!url.pathname.startsWith("/files/")) {
    return;
  }
  requireMethod(req, "GET");
  if (url.pathname === "/files/tree") {
    const root = url.searchParams.get("root");
    if (!root) {
      throw new ApiError(400, "root query parameter is required");
    }
    const relativePath = url.searchParams.get("path") ?? "";
    sendJson(res, 200, await listLiveFiles(root, relativePath));
    return;
  }
  if (url.pathname === "/files/read") {
    const filePath = url.searchParams.get("path");
    if (!filePath) {
      throw new ApiError(400, "path query parameter is required");
    }
    sendJson(res, 200, await readLiveFile(filePath));
    return;
  }
  throw new ApiError(404, `Unknown file route: ${url.pathname}`);
}

async function profilesRoute(req: IncomingMessage, res: ServerResponse, url: URL): Promise<void> {
  if (!url.pathname.startsWith("/profiles")) {
    return;
  }
  if (route(url.pathname, req.method ?? "GET", "/profiles", "GET")) {
    sendJson(res, 200, await listProfiles());
    return;
  }
  if (route(url.pathname, req.method ?? "POST", "/profiles/create", "POST")) {
    const body = await readJsonBody<{ name?: string; modelAlias?: string }>(req);
    sendJson(res, 200, await createProfile(body.name ?? "", body.modelAlias ?? ""));
    return;
  }
  if (route(url.pathname, req.method ?? "POST", "/profiles/embed", "POST")) {
    const body = await readJsonBody<{ name?: string }>(req);
    sendJson(res, 200, await embedProfile(body.name ?? ""));
    return;
  }
  if (route(url.pathname, req.method ?? "POST", "/profiles/query", "POST")) {
    const body = await readJsonBody<{ name?: string; query?: string; limit?: number }>(req);
    sendJson(res, 200, await queryProfile(body.name ?? "", body.query ?? "", body.limit ?? 10));
    return;
  }
  throw new ApiError(404, `Unknown profile route: ${url.pathname}`);
}

export function createPlaygroundApiMiddleware(): Middleware {
  ensureManagedRuntimeStarted();
  return (req, res, next) => {
    handleRequest(req, res).catch((error) => {
      if (!res.writableEnded) {
        sendError(res, error);
        return;
      }
      next(error);
    });
  };
}
