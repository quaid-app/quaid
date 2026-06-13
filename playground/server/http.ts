import type { IncomingMessage, ServerResponse } from "node:http";
import type { ApiErrorShape } from "./types";

export type Middleware = (
  req: IncomingMessage,
  res: ServerResponse,
  next: (error?: unknown) => void
) => void;

export class ApiError extends Error {
  readonly status: number;
  readonly details?: unknown;

  constructor(status: number, message: string, details?: unknown) {
    super(message);
    this.status = status;
    this.details = details;
  }
}

export function sendJson(res: ServerResponse, status: number, value: unknown): void {
  const body = JSON.stringify(value, null, 2);
  res.statusCode = status;
  res.setHeader("content-type", "application/json; charset=utf-8");
  res.setHeader("cache-control", "no-store");
  res.end(body);
}

export function sendText(res: ServerResponse, status: number, value: string, contentType = "text/plain; charset=utf-8"): void {
  res.statusCode = status;
  res.setHeader("content-type", contentType);
  res.setHeader("cache-control", "no-store");
  res.end(value);
}

export function sendError(res: ServerResponse, error: unknown): void {
  const apiError = normalizeError(error);
  const body: ApiErrorShape = {
    error: apiError.message,
    details: apiError.details
  };
  sendJson(res, apiError.status, body);
}

export function normalizeError(error: unknown): ApiError {
  if (error instanceof ApiError) {
    return error;
  }
  if (error instanceof Error) {
    return new ApiError(500, error.message);
  }
  return new ApiError(500, String(error));
}

export async function readJsonBody<T = unknown>(req: IncomingMessage): Promise<T> {
  assertJsonContentType(req);
  const chunks: Buffer[] = [];
  for await (const chunk of req) {
    chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
  }
  const raw = Buffer.concat(chunks).toString("utf8").trim();
  if (!raw) {
    return {} as T;
  }
  try {
    return JSON.parse(raw) as T;
  } catch (error) {
    throw new ApiError(400, "Request body must be valid JSON", error);
  }
}

export function requireMethod(req: IncomingMessage, method: string): void {
  if (req.method !== method) {
    throw new ApiError(405, `Expected ${method}, got ${req.method ?? "UNKNOWN"}`);
  }
}

export function parseUrl(req: IncomingMessage): URL {
  return new URL(req.url ?? "/", "http://127.0.0.1");
}

const BODIED_METHODS = new Set(["POST", "PUT", "PATCH", "DELETE"]);
const LOOPBACK_HOSTNAMES = new Set(["127.0.0.1", "localhost", "[::1]", "::1"]);

/**
 * Require `application/json` for any request method that carries a body. This
 * forces a cross-origin POST out of the CORS "simple request" lane (which
 * allows `text/plain` without a preflight) and into a preflighted request that
 * the playground answers with no CORS headers — so the browser blocks it.
 */
export function assertJsonContentType(req: IncomingMessage): void {
  const method = (req.method ?? "GET").toUpperCase();
  if (!BODIED_METHODS.has(method)) {
    return;
  }
  const contentType = req.headers["content-type"] ?? "";
  const mediaType = contentType.split(";", 1)[0].trim().toLowerCase();
  if (mediaType !== "application/json") {
    throw new ApiError(
      415,
      `Expected content-type application/json, got ${mediaType || "(none)"}`
    );
  }
}

/**
 * A host header (with optional port) is allowed only when its hostname is a
 * loopback name. This blocks DNS-rebinding origins that resolve a public name
 * to 127.0.0.1 — the browser still sends the rebound `Host`, which we reject.
 */
export function isAllowedHost(hostHeader: string | undefined): boolean {
  if (!hostHeader) {
    // Same-origin loopback requests always carry a Host header; its absence is
    // anomalous, so reject.
    return false;
  }
  let hostname: string;
  try {
    hostname = new URL(`http://${hostHeader}`).hostname;
  } catch {
    return false;
  }
  return LOOPBACK_HOSTNAMES.has(hostname.toLowerCase());
}

/**
 * An Origin/Referer is allowed only when its hostname is a loopback name. Same-
 * origin browser fetches to `/api` send no cross-origin Origin (and a loopback
 * Referer at most), and server-to-server callers omit Origin entirely — both
 * pass. A foreign site's `fetch` carries its own Origin, which we reject.
 */
export function isAllowedOrigin(originHeader: string | undefined): boolean {
  if (!originHeader || originHeader === "null") {
    return false;
  }
  let hostname: string;
  try {
    hostname = new URL(originHeader).hostname;
  } catch {
    return false;
  }
  return LOOPBACK_HOSTNAMES.has(hostname.toLowerCase());
}

/**
 * Reject cross-origin and DNS-rebinding callers. Only enforce the Origin/
 * Referer check when one is present, so legitimate same-origin browser calls
 * (no cross-origin Origin) and server-to-server callers (no Origin) still pass.
 * The Host check always runs to defeat DNS rebinding on GET reads.
 */
export function assertSameOrigin(req: IncomingMessage): void {
  if (!isAllowedHost(req.headers.host)) {
    throw new ApiError(403, `Forbidden host: ${req.headers.host ?? "(none)"}`);
  }
  const origin = req.headers.origin;
  if (origin != null && !isAllowedOrigin(origin)) {
    throw new ApiError(403, `Forbidden origin: ${origin}`);
  }
  const referer = req.headers.referer;
  if (origin == null && referer != null && !isAllowedOrigin(referer)) {
    throw new ApiError(403, `Forbidden referer: ${referer}`);
  }
}
