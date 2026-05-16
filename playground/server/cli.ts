import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { getConfig } from "./config";
import { ApiError } from "./http";
import type { CliCommandSpec, CliRunRequest, CliRunResult } from "./types";

const execFileAsync = promisify(execFile);

export const CLI_COMMANDS: CliCommandSpec[] = [
  {
    id: "model.pull",
    label: "Download SLM model",
    description: "Runs quaid model pull for a fact-extraction SLM alias.",
    risky: true,
    params: [{ name: "alias", label: "Model alias", required: true, defaultValue: "phi-3.5-mini" }]
  },
  {
    id: "extraction.status",
    label: "Extraction status",
    description: "Shows fact extraction runtime and queue status.",
    risky: false,
    params: []
  },
  {
    id: "extraction.enable",
    label: "Enable extraction",
    description: "Enables conversation fact extraction for the active DB.",
    risky: true,
    params: []
  },
  {
    id: "extraction.disable",
    label: "Disable extraction",
    description: "Disables conversation fact extraction for the active DB.",
    risky: true,
    params: []
  },
  {
    id: "config.set",
    label: "Set config value",
    description: "Sets one Quaid config key in the active DB.",
    risky: true,
    params: [
      { name: "key", label: "Key", required: true, defaultValue: "extraction.model_alias" },
      { name: "value", label: "Value", required: true, defaultValue: "phi-3.5-mini" }
    ]
  },
  {
    id: "config.get",
    label: "Get config value",
    description: "Reads one Quaid config key from the active DB.",
    risky: false,
    params: [{ name: "key", label: "Key", required: true, defaultValue: "extraction.model_alias" }]
  },
  {
    id: "extract.session",
    label: "Extract session",
    description: "Enqueues fact extraction for one conversation session.",
    risky: true,
    params: [
      { name: "sessionId", label: "Session ID", required: true },
      { name: "force", label: "Force", type: "boolean", defaultValue: false }
    ]
  },
  {
    id: "extract.all",
    label: "Extract all",
    description: "Enqueues fact extraction across all conversation sessions.",
    risky: true,
    params: [{ name: "force", label: "Force", type: "boolean", defaultValue: false }]
  },
  {
    id: "embed.all",
    label: "Embed all",
    description: "Refreshes all embeddings in the active DB.",
    risky: true,
    params: []
  },
  {
    id: "embed.stale",
    label: "Embed stale",
    description: "Refreshes stale embeddings in the active DB.",
    risky: true,
    params: []
  },
  {
    id: "status.json",
    label: "Runtime status",
    description: "Runs quaid status --json.",
    risky: false,
    params: []
  },
  {
    id: "stats.json",
    label: "Memory stats",
    description: "Runs quaid --json stats.",
    risky: false,
    params: []
  },
  {
    id: "mcp.call",
    label: "Raw MCP tool call",
    description: "Calls an existing Quaid MCP tool through quaid call.",
    risky: true,
    params: [
      { name: "tool", label: "Tool", required: true, defaultValue: "memory_stats" },
      { name: "params", label: "JSON params", type: "json", defaultValue: "{}" }
    ]
  }
];

const specsById = new Map(CLI_COMMANDS.map((command) => [command.id, command]));

function stringParam(params: Record<string, unknown>, name: string, fallback = ""): string {
  const value = params[name];
  if (value == null || value === "") {
    return fallback;
  }
  return String(value);
}

function boolParam(params: Record<string, unknown>, name: string, fallback = false): boolean {
  const value = params[name];
  if (value == null || value === "") {
    return fallback;
  }
  if (typeof value === "boolean") {
    return value;
  }
  return ["1", "true", "yes", "on"].includes(String(value).toLowerCase());
}

function jsonParam(params: Record<string, unknown>, name: string): string {
  const value = params[name];
  if (typeof value === "string") {
    JSON.parse(value);
    return value;
  }
  return JSON.stringify(value ?? {});
}

export function buildCliArgs(request: CliRunRequest): string[] {
  const config = getConfig();
  const params = request.params ?? {};
  const base = ["--db", config.dbPath, "--model", config.embeddingModel];

  switch (request.id) {
    case "model.pull":
      return ["model", "pull", stringParam(params, "alias", "phi-3.5-mini")];
    case "extraction.status":
      return [...base, "extraction", "status"];
    case "extraction.enable":
      return [...base, "extraction", "enable"];
    case "extraction.disable":
      return [...base, "extraction", "disable"];
    case "config.set":
      return [...base, "config", "set", stringParam(params, "key"), stringParam(params, "value")];
    case "config.get":
      return [...base, "config", "get", stringParam(params, "key")];
    case "extract.session": {
      const args = [...base, "extract", stringParam(params, "sessionId")];
      if (boolParam(params, "force")) {
        args.push("--force");
      }
      return args;
    }
    case "extract.all": {
      const args = [...base, "extract", "--all"];
      if (boolParam(params, "force")) {
        args.push("--force");
      }
      return args;
    }
    case "embed.all":
      return [...base, "embed", "--all"];
    case "embed.stale":
      return [...base, "embed", "--stale"];
    case "status.json":
      return [...base, "status", "--json"];
    case "stats.json":
      return [...base, "--json", "stats"];
    case "mcp.call":
      return [
        ...base,
        "call",
        stringParam(params, "tool", "memory_stats"),
        jsonParam(params, "params")
      ];
    default:
      throw new ApiError(400, `Unknown CLI command id: ${request.id}`);
  }
}

function parseStdout(stdout: string): unknown | undefined {
  const trimmed = stdout.trim();
  if (!trimmed) {
    return undefined;
  }
  try {
    return JSON.parse(trimmed);
  } catch {
    return undefined;
  }
}

export async function runCliCommand(request: CliRunRequest): Promise<CliRunResult> {
  const spec = specsById.get(request.id);
  if (!spec) {
    throw new ApiError(400, `Unknown CLI command id: ${request.id}`);
  }
  if (spec.risky && request.confirmed !== true) {
    throw new ApiError(409, `Command requires confirmation: ${request.id}`);
  }

  const args = buildCliArgs(request);
  const config = getConfig();

  try {
    const result = await execFileAsync(config.quaidBin, args, {
      cwd: config.rootDir,
      windowsHide: true,
      timeout: 10 * 60 * 1000,
      maxBuffer: 20 * 1024 * 1024
    });
    return {
      id: request.id,
      args,
      stdout: result.stdout,
      stderr: result.stderr,
      exitCode: 0,
      parsed: parseStdout(result.stdout)
    };
  } catch (error) {
    const err = error as Error & {
      stdout?: string;
      stderr?: string;
      code?: number;
    };
    return {
      id: request.id,
      args,
      stdout: err.stdout ?? "",
      stderr: err.stderr ?? err.message,
      exitCode: typeof err.code === "number" ? err.code : 1,
      parsed: parseStdout(err.stdout ?? "")
    };
  }
}

export async function callToolViaCli(tool: string, params: unknown): Promise<unknown> {
  const result = await runCliCommand({
    id: "mcp.call",
    params: {
      tool,
      params
    },
    confirmed: true
  });
  if (result.exitCode !== 0) {
    throw new ApiError(502, `quaid call failed for ${tool}`, {
      stdout: result.stdout,
      stderr: result.stderr
    });
  }
  return result.parsed ?? result.stdout;
}
