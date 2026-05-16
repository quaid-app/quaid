import type { CliRunResult } from "./types";

async function parseResponse<T>(response: Response): Promise<T> {
  const contentType = response.headers.get("content-type") ?? "";
  const value = contentType.includes("application/json") ? await response.json() : await response.text();
  if (!response.ok) {
    const message = typeof value === "object" && value && "error" in value ? String(value.error) : response.statusText;
    throw new Error(message);
  }
  return value as T;
}

export async function apiGet<T>(path: string): Promise<T> {
  const response = await fetch(`/api${path}`, {
    headers: { accept: "application/json" }
  });
  return parseResponse<T>(response);
}

export async function apiPost<T>(path: string, body: unknown = {}): Promise<T> {
  const response = await fetch(`/api${path}`, {
    method: "POST",
    headers: {
      accept: "application/json",
      "content-type": "application/json"
    },
    body: JSON.stringify(body)
  });
  return parseResponse<T>(response);
}

export async function callMcp<T = unknown>(name: string, args: unknown = {}): Promise<T> {
  return apiPost<T>("/mcp/call", { name, args });
}

export async function runCli(id: string, params: Record<string, unknown> = {}, confirmed = false): Promise<CliRunResult> {
  return apiPost<CliRunResult>("/cli/run", { id, params, confirmed });
}

export function prettyJson(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  return JSON.stringify(value, null, 2);
}
