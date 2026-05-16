import { callMcp } from "./api";
import type { EnrichedResult, MemoryPage, SearchResult } from "./types";

export function normalizeSearchResults(value: unknown): SearchResult[] {
  if (Array.isArray(value)) {
    return value.filter(isSearchResult);
  }
  if (value && typeof value === "object") {
    const record = value as Record<string, unknown>;
    for (const key of ["results", "items", "pages"]) {
      if (Array.isArray(record[key])) {
        return record[key].filter(isSearchResult);
      }
    }
  }
  return [];
}

function isSearchResult(value: unknown): value is SearchResult {
  return Boolean(value && typeof value === "object" && typeof (value as { slug?: unknown }).slug === "string");
}

export async function queryAndEnrich(query: string, limit: number, namespace?: string): Promise<EnrichedResult[]> {
  const raw = await callMcp("memory_query", {
    query,
    limit,
    depth: "auto",
    namespace: namespace || undefined
  });
  const results = normalizeSearchResults(raw);
  return Promise.all(
    results.map(async (result) => {
      try {
        const page = await callMcp<MemoryPage>("memory_get", { slug: result.slug });
        return { result, page };
      } catch (error) {
        return {
          result,
          error: error instanceof Error ? error.message : String(error)
        };
      }
    })
  );
}

export function composeRetrievalAnswer(question: string, items: EnrichedResult[]): string {
  if (items.length === 0) {
    return "No matching memories were retrieved. Try a broader query or check whether extraction has produced facts for this conversation.";
  }

  const snippets = items
    .slice(0, 3)
    .map((item) => {
      const page = item.page;
      const source = page?.title || item.result.title || item.result.slug;
      const text = [page?.summary, page?.compiled_truth, item.result.summary]
        .filter(Boolean)
        .join(" ")
        .replace(/\s+/g, " ")
        .slice(0, 240);
      return `${source}: ${text}`;
    })
    .filter((line) => !line.endsWith(": "));

  if (snippets.length === 0) {
    return `Retrieved ${items.length} result${items.length === 1 ? "" : "s"} for "${question}", but none exposed a summary or compiled fact body.`;
  }

  return `Retrieved evidence for "${question}" points to: ${snippets.join(" ")}`
}
