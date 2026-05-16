import { describe, expect, it } from "vitest";
import { composeRetrievalAnswer, normalizeSearchResults } from "../../src/lib/search";

describe("search helpers", () => {
  it("normalizes direct result arrays", () => {
    expect(normalizeSearchResults([{ slug: "concept/a" }, { nope: true }])).toEqual([{ slug: "concept/a" }]);
  });

  it("builds a retrieval answer from enriched snippets", () => {
    const answer = composeRetrievalAnswer("What do I drink?", [
      {
        result: { slug: "conversation/session", score: 0.8 },
        page: {
          slug: "conversation/session",
          title: "Session",
          type: "source",
          summary: "The user prefers coffee over tea.",
          compiled_truth: "",
          timeline: "",
          wing: "",
          room: "",
          version: 1,
          updated_at: "2026-05-16T00:00:00Z"
        }
      }
    ]);
    expect(answer).toContain("coffee");
  });
});
