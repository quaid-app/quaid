import { execFileSync, spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { existsSync, mkdirSync } from "node:fs";
import { dirname } from "node:path";
import { getConfig } from "./config";
import { ApiError } from "./http";

let managedProcess: ChildProcessWithoutNullStreams | null = null;
const logLines: string[] = [];
let autoStartAttempted = false;

function remember(line: string): void {
  const lines = line
    .split(/\r?\n/)
    .map((part) => part.trimEnd())
    .filter(Boolean);
  for (const clean of lines) {
    logLines.push(clean);
    console.info(`[quaid-runtime] ${clean}`);
  }
  while (logLines.length > 200) {
    logLines.shift();
  }
}

export function runtimeStatus() {
  const config = getConfig();
  return {
    mode: config.runtimeMode,
    mcpUrl: config.mcpUrl,
    managedRunning: Boolean(managedProcess && !managedProcess.killed),
    pid: managedProcess?.pid ?? null,
    logTail: logLines.slice(-40)
  };
}

export function startRuntime() {
  const config = getConfig();
  if (config.runtimeMode === "external") {
    throw new ApiError(409, "Runtime is external; start Quaid outside the playground.");
  }
  if (managedProcess && !managedProcess.killed) {
    return runtimeStatus();
  }

  bootstrapManagedRuntime();

  const args = [
    "--db",
    config.dbPath,
    "--model",
    config.embeddingModel,
    "serve",
    "--http",
    "--port",
    String(config.httpPort),
    "--trust-loopback"
  ];

  managedProcess = spawn(config.quaidBin, args, {
    cwd: config.rootDir,
    windowsHide: true,
    env: {
      ...process.env,
      QUAID_DB: config.dbPath,
      QUAID_MODEL: config.embeddingModel
    }
  });

  remember(`started: ${config.quaidBin} ${args.join(" ")}`);
  managedProcess.stdout.on("data", (chunk) => remember(String(chunk)));
  managedProcess.stderr.on("data", (chunk) => remember(String(chunk)));
  managedProcess.on("error", (error) => {
    remember(`quaid serve failed to start: ${error.message}`);
    managedProcess = null;
  });
  managedProcess.on("exit", (code, signal) => {
    remember(`quaid serve exited code=${code ?? "null"} signal=${signal ?? "null"}`);
    managedProcess = null;
  });

  return runtimeStatus();
}

function bootstrapManagedRuntime(): void {
  const config = getConfig();
  const env = {
    ...process.env,
    QUAID_DB: config.dbPath,
    QUAID_MODEL: config.embeddingModel
  };

  if (config.autoInitDb && !existsSync(config.dbPath)) {
    const parent = dirname(config.dbPath);
    mkdirSync(parent, { recursive: true });
    remember(`initializing missing DB: ${config.dbPath}`);
    runQuaidBootstrap(["--model", config.embeddingModel, "init", config.dbPath], env);
  }

  if (config.autoEnableExtraction) {
    remember("auto-enabling fact extraction");
    runQuaidBootstrap(
      ["--db", config.dbPath, "--model", config.embeddingModel, "extraction", "enable"],
      env
    );
  }
}

function runQuaidBootstrap(args: string[], env: NodeJS.ProcessEnv): void {
  const config = getConfig();
  try {
    const output = execFileSync(config.quaidBin, args, {
      cwd: config.rootDir,
      env,
      encoding: "utf8",
      windowsHide: true,
      timeout: 10 * 60 * 1000,
      maxBuffer: 20 * 1024 * 1024
    });
    remember(output || `completed: ${config.quaidBin} ${args.join(" ")}`);
  } catch (error) {
    const err = error as Error & { stdout?: string; stderr?: string };
    remember(err.stdout ?? "");
    remember(err.stderr ?? "");
    throw new ApiError(502, `Quaid bootstrap failed: ${err.message}`);
  }
}

export function stopRuntime() {
  if (managedProcess && !managedProcess.killed) {
    managedProcess.kill();
    remember("stop requested");
  }
  managedProcess = null;
  return runtimeStatus();
}

export function restartRuntime() {
  stopRuntime();
  return startRuntime();
}

export function ensureManagedRuntimeStarted() {
  const config = getConfig();
  if (config.runtimeMode === "external" || autoStartAttempted) {
    return runtimeStatus();
  }
  autoStartAttempted = true;
  try {
    return startRuntime();
  } catch (error) {
    remember(`quaid serve auto-start failed: ${error instanceof Error ? error.message : String(error)}`);
    return runtimeStatus();
  }
}
