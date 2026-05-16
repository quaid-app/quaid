import fs from "node:fs/promises";
import path from "node:path";
import { getCollections } from "./db";
import { ApiError } from "./http";
import { isPathInside } from "./pathSafety";
import type { TreeNode } from "./types";

const MAX_FILE_BYTES = 2 * 1024 * 1024;

export async function listLiveFiles(rootPath: string, relativePath = "", depth = 3): Promise<TreeNode> {
  const root = await allowedRoot(rootPath);
  const target = path.resolve(root, relativePath);
  if (!isPathInside(root, target)) {
    throw new ApiError(403, "Requested path is outside the selected collection root");
  }
  return readDirNode(root, target, depth);
}

export async function readLiveFile(filePath: string) {
  const roots = await allowedRoots();
  const target = path.resolve(filePath);
  const root = roots.find((candidate) => isPathInside(candidate, target));
  if (!root) {
    throw new ApiError(403, "File is outside all active collection roots");
  }
  const stats = await fs.stat(target);
  if (!stats.isFile()) {
    throw new ApiError(400, "Path is not a file");
  }
  if (stats.size > MAX_FILE_BYTES) {
    throw new ApiError(413, "File is too large for the playground viewer");
  }
  return {
    path: target,
    size: stats.size,
    modifiedAt: stats.mtime.toISOString(),
    content: await fs.readFile(target, "utf8")
  };
}

async function allowedRoot(rootPath: string): Promise<string> {
  const roots = await allowedRoots();
  const resolved = path.resolve(rootPath);
  const match = roots.find((root) => path.resolve(root) === resolved);
  if (!match) {
    throw new ApiError(403, "Unknown collection root");
  }
  return match;
}

async function allowedRoots(): Promise<string[]> {
  return getCollections()
    .map((collection) => String(collection.root_path ?? "").trim())
    .filter(Boolean)
    .map((root) => path.resolve(root));
}

async function readDirNode(root: string, target: string, depth: number): Promise<TreeNode> {
  const stats = await fs.stat(target);
  if (stats.isFile()) {
    return {
      id: target,
      label: path.basename(target),
      path: target,
      type: "file",
      meta: {
        size: stats.size,
        modifiedAt: stats.mtime.toISOString(),
        relativePath: path.relative(root, target)
      }
    };
  }

  const entries = await fs.readdir(target, { withFileTypes: true });
  const children: TreeNode[] = [];
  for (const entry of entries.sort((a, b) => Number(b.isDirectory()) - Number(a.isDirectory()) || a.name.localeCompare(b.name))) {
    if (entry.name.startsWith(".git")) {
      continue;
    }
    const childPath = path.join(target, entry.name);
    if (entry.isDirectory()) {
      children.push(
        depth > 0
          ? await readDirNode(root, childPath, depth - 1)
          : {
              id: childPath,
              label: entry.name,
              path: childPath,
              type: "directory",
              children: []
            }
      );
    } else if (entry.isFile()) {
      const childStats = await fs.stat(childPath);
      children.push({
        id: childPath,
        label: entry.name,
        path: childPath,
        type: "file",
        meta: {
          size: childStats.size,
          modifiedAt: childStats.mtime.toISOString(),
          relativePath: path.relative(root, childPath)
        }
      });
    }
  }

  return {
    id: target,
    label: path.basename(target) || target,
    path: target,
    type: target === root ? "root" : "directory",
    children
  };
}
