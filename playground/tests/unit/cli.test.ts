import { describe, expect, it } from "vitest";
import { buildCliArgs, CLI_COMMANDS } from "../../server/cli";

describe("CLI allowlist", () => {
  it("exposes only known command ids", () => {
    expect(CLI_COMMANDS.map((command) => command.id)).toEqual(
      expect.arrayContaining(["model.pull", "extraction.status", "config.set", "mcp.call"])
    );
  });

  it("builds model pull without a database flag", () => {
    expect(buildCliArgs({ id: "model.pull", params: { alias: "phi-3.5-mini" }, confirmed: true })).toEqual([
      "model",
      "pull",
      "phi-3.5-mini"
    ]);
  });

  it("validates raw MCP params JSON before building args", () => {
    expect(() =>
      buildCliArgs({
        id: "mcp.call",
        params: { tool: "memory_stats", params: "{not-json" },
        confirmed: true
      })
    ).toThrow();
  });
});
