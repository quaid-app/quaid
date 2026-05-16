import { FileText } from "lucide-react";
import type { EnrichedResult } from "../lib/types";

export function ResultCard({ item }: { item: EnrichedResult }) {
  const page = item.page;
  const result = item.result;
  const title = page?.title || result.title || result.slug;
  const summary = page?.summary || result.summary || page?.compiled_truth || "No summary available.";

  return (
    <article className="result-card">
      <header>
        <FileText aria-hidden="true" size={18} />
        <div>
          <h3>{title}</h3>
          <span>{page?.canonicalSlug || result.slug}</span>
        </div>
      </header>
      <p>{summary}</p>
      <dl className="metadata-grid">
        <div>
          <dt>Type</dt>
          <dd>{page?.type || result.type || "-"}</dd>
        </div>
        <div>
          <dt>Score</dt>
          <dd>{typeof result.score === "number" ? result.score.toFixed(3) : "-"}</dd>
        </div>
        <div>
          <dt>Wing</dt>
          <dd>{page?.wing || result.wing || "-"}</dd>
        </div>
        <div>
          <dt>Version</dt>
          <dd>{page?.version ?? "-"}</dd>
        </div>
      </dl>
      {item.error ? <p className="inline-error">{item.error}</p> : null}
    </article>
  );
}
