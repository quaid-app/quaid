import fs from "node:fs/promises";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { getConfig } from "./config";
import { ApiError } from "./http";
import { safeSegment } from "./pathSafety";

const execFileAsync = promisify(execFile);

export interface EmbeddingProfile {
  name: string;
  modelAlias: string;
  dbPath: string;
  createdAt: string;
}

async function profilePath(name: string): Promise<string> {
  const config = getConfig();
  const dir = path.join(config.profilesDir, safeSegment(name));
  await fs.mkdir(dir, { recursive: true });
  return dir;
}

async function metadataPath(name: string): Promise<string> {
  return path.join(await profilePath(name), "profile.json");
}

export async function listProfiles(): Promise<EmbeddingProfile[]> {
  const config = getConfig();
  try {
    const entries = await fs.readdir(config.profilesDir, { withFileTypes: true });
    const profiles: EmbeddingProfile[] = [];
    for (const entry of entries) {
      if (!entry.isDirectory()) {
        continue;
      }
      const metadataFile = path.join(config.profilesDir, entry.name, "profile.json");
      try {
        const raw = await fs.readFile(metadataFile, "utf8");
        profiles.push(JSON.parse(raw) as EmbeddingProfile);
      } catch {
        continue;
      }
    }
    return profiles.sort((a, b) => a.name.localeCompare(b.name));
  } catch {
    return [];
  }
}

export async function createProfile(name: string, modelAlias: string): Promise<EmbeddingProfile> {
  if (!name.trim()) {
    throw new ApiError(400, "Profile name is required");
  }
  if (!modelAlias.trim()) {
    throw new ApiError(400, "Embedding model alias is required");
  }

  const config = getConfig();
  const dir = await profilePath(name);
  const dbPath = path.join(dir, "memory.db");
  const metadata: EmbeddingProfile = {
    name,
    modelAlias,
    dbPath,
    createdAt: new Date().toISOString()
  };

  try {
    await execFileAsync(config.quaidBin, ["--model", modelAlias, "init", dbPath], {
      cwd: config.rootDir,
      windowsHide: true,
      timeout: 120_000,
      maxBuffer: 10 * 1024 * 1024
    });
    await fs.writeFile(await metadataPath(name), JSON.stringify(metadata, null, 2), "utf8");
    return metadata;
  } catch (error) {
    const err = error as Error & { stderr?: string; stdout?: string };
    throw new ApiError(502, `Failed to create embedding profile: ${err.message}`, {
      stdout: err.stdout,
      stderr: err.stderr
    });
  }
}

export async function embedProfile(name: string) {
  const profile = await loadProfile(name);
  const config = getConfig();
  try {
    const result = await execFileAsync(
      config.quaidBin,
      ["--db", profile.dbPath, "--model", profile.modelAlias, "embed", "--all"],
      {
        cwd: config.rootDir,
        windowsHide: true,
        timeout: 10 * 60 * 1000,
        maxBuffer: 20 * 1024 * 1024
      }
    );
    return {
      profile,
      stdout: result.stdout,
      stderr: result.stderr
    };
  } catch (error) {
    const err = error as Error & { stderr?: string; stdout?: string };
    throw new ApiError(502, `Failed to embed profile: ${err.message}`, {
      stdout: err.stdout,
      stderr: err.stderr
    });
  }
}

export async function queryProfile(name: string, query: string, limit = 10) {
  const profile = await loadProfile(name);
  const config = getConfig();
  try {
    const result = await execFileAsync(
      config.quaidBin,
      [
        "--db",
        profile.dbPath,
        "--model",
        profile.modelAlias,
        "--json",
        "query",
        query,
        "--limit",
        String(limit)
      ],
      {
        cwd: config.rootDir,
        windowsHide: true,
        timeout: 120_000,
        maxBuffer: 20 * 1024 * 1024
      }
    );
    const parsed = JSON.parse(result.stdout || "[]");
    return {
      profile,
      results: parsed,
      stdout: result.stdout,
      stderr: result.stderr
    };
  } catch (error) {
    const err = error as Error & { stderr?: string; stdout?: string };
    throw new ApiError(502, `Failed to query profile: ${err.message}`, {
      stdout: err.stdout,
      stderr: err.stderr
    });
  }
}

async function loadProfile(name: string): Promise<EmbeddingProfile> {
  const file = await metadataPath(name);
  try {
    const raw = await fs.readFile(file, "utf8");
    return JSON.parse(raw) as EmbeddingProfile;
  } catch {
    throw new ApiError(404, `Embedding profile not found: ${name}`);
  }
}
