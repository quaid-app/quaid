import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ResultCard } from "../../src/components/ResultCard";

describe("ResultCard", () => {
  it("renders result metadata", () => {
    render(
      <ResultCard
        item={{
          result: { slug: "people/alice", score: 0.91 },
          page: {
            slug: "people/alice",
            canonicalSlug: "default::people/alice",
            title: "Alice",
            type: "person",
            summary: "Knows the retrieval system.",
            compiled_truth: "",
            timeline: "",
            wing: "work",
            room: "",
            version: 2,
            updated_at: "2026-05-16T00:00:00Z"
          }
        }}
      />
    );

    expect(screen.getByText("Alice")).toBeInTheDocument();
    expect(screen.getByText("0.910")).toBeInTheDocument();
    expect(screen.getByText("person")).toBeInTheDocument();
  });
});
