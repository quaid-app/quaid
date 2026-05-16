import path from "node:path";

export function normalizeForCompare(value: string): string {
  const resolved = path.resolve(value);
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
}

export function isPathInside(root: string, candidate: string): boolean {
  const normalizedRoot = normalizeForCompare(root);
  const normalizedCandidate = normalizeForCompare(candidate);
  if (normalizedRoot === normalizedCandidate) {
    return true;
  }
  const relative = path.relative(normalizedRoot, normalizedCandidate);
  return Boolean(relative) && !relative.startsWith("..") && !path.isAbsolute(relative);
}

export function assertPathInside(root: string, candidate: string): string {
  const resolved = path.resolve(candidate);
  if (!isPathInside(root, resolved)) {
    throw new Error(`Path is outside allowed root: ${candidate}`);
  }
  return resolved;
}

export function safeSegment(value: string): string {
  return value.replace(/[^a-zA-Z0-9._-]/g, "_").slice(0, 80) || "profile";
}
