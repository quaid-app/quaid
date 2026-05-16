import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import type { PlaygroundConfig } from "./types";

const serverDir = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(serverDir, "..");

function defaultDbPath(): string {
  return path.join(os.homedir(), ".quaid", "memory.db");
}

function envFlag(name: string, defaultValue: boolean): boolean {
  const raw = process.env[name];
  if (raw == null || raw === "") {
    return defaultValue;
  }
  return ["1", "true", "yes", "on"].includes(raw.toLowerCase());
}

export function getConfig(): PlaygroundConfig {
  const httpPort = Number(process.env.QUAID_HTTP_PORT ?? "3112");
  const explicitMcpUrl = process.env.QUAID_MCP_URL?.trim();
  const mcpUrl = explicitMcpUrl || `http://127.0.0.1:${httpPort}`;

  return {
    rootDir,
    profilesDir: path.join(rootDir, ".quaid-profiles"),
    quaidBin: process.env.QUAID_BIN?.trim() || "quaid",
    dbPath: process.env.QUAID_DB?.trim() || defaultDbPath(),
    embeddingModel: process.env.QUAID_MODEL?.trim() || "small",
    httpPort,
    mcpUrl,
    runtimeMode: explicitMcpUrl ? "external" : "managed",
    cliFallbackForMcp: envFlag("QUAID_PLAYGROUND_MCP_CLI_FALLBACK", true),
    autoInitDb: envFlag("QUAID_PLAYGROUND_AUTO_INIT", true),
    autoEnableExtraction: envFlag("QUAID_PLAYGROUND_AUTO_ENABLE_EXTRACTION", false)
  };
}

export function publicConfig() {
  const config = getConfig();
  return {
    rootDir: config.rootDir,
    profilesDir: config.profilesDir,
    quaidBin: config.quaidBin,
    dbPath: config.dbPath,
    embeddingModel: config.embeddingModel,
    httpPort: config.httpPort,
    mcpUrl: config.mcpUrl,
    runtimeMode: config.runtimeMode,
    cliFallbackForMcp: config.cliFallbackForMcp,
    autoInitDb: config.autoInitDb,
    autoEnableExtraction: config.autoEnableExtraction
  };
}
