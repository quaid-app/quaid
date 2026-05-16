import path from "node:path";
import { describe, expect, it } from "vitest";
import { isPathInside, safeSegment } from "../../server/pathSafety";

describe("path safety", () => {
  it("allows the root itself and descendants", () => {
    const root = path.resolve("vault");
    expect(isPathInside(root, root)).toBe(true);
    expect(isPathInside(root, path.join(root, "notes", "a.md"))).toBe(true);
  });

  it("rejects sibling paths", () => {
    const root = path.resolve("vault");
    expect(isPathInside(root, path.resolve("vault-other", "a.md"))).toBe(false);
  });

  it("sanitizes profile names into path segments", () => {
    expect(safeSegment("../base model")).toBe(".._base_model");
  });
});
