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
