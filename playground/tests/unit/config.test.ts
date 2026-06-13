import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getConfig } from "../../server/config";

describe("playground default DB path", () => {
  let savedDb: string | undefined;

  beforeEach(() => {
    savedDb = process.env.QUAID_DB;
    delete process.env.QUAID_DB;
  });

  afterEach(() => {
    if (savedDb === undefined) {
      delete process.env.QUAID_DB;
    } else {
      process.env.QUAID_DB = savedDb;
    }
  });

  it("defaults to a throwaway DB inside the playground repo, not ~/.quaid", () => {
    const config = getConfig();

    // The default must live under the playground root, never the developer's
    // real ~/.quaid/memory.db.
    expect(config.dbPath).toBe(path.join(config.rootDir, ".data", "memory.db"));

    const relative = path.relative(config.rootDir, config.dbPath);
    expect(relative.startsWith("..")).toBe(false);
    expect(path.isAbsolute(relative)).toBe(false);
    expect(config.dbPath).not.toContain(".quaid/memory.db");
  });

  it("still honors an explicit QUAID_DB override", () => {
    process.env.QUAID_DB = "/tmp/custom-quaid.db";
    expect(getConfig().dbPath).toBe("/tmp/custom-quaid.db");
  });
});
