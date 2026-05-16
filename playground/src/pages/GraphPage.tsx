import { useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import ForceGraph2D from "react-force-graph-2d";
import { FormField } from "../components/FormField";
import { apiGet } from "../lib/api";
import type { GraphData } from "../lib/types";

type ForceNode = {
  id?: string;
  label?: string;
  title?: string;
  type?: string;
  x?: number;
  y?: number;
};

type ForceLink = {
  label?: string;
};

export function GraphPage() {
  const [scope, setScope] = useState<"whole" | "focused">("whole");
  const [slug, setSlug] = useState("");
  const [depth, setDepth] = useState(2);

  const query = useQuery<GraphData>({
    queryKey: ["graph", scope, slug, depth],
    queryFn: () => {
      const params = new URLSearchParams({ scope, depth: String(depth) });
      if (slug) {
        params.set("slug", slug);
      }
      return apiGet(`/db/graph?${params.toString()}`);
    },
    enabled: scope === "whole" || Boolean(slug)
  });

  const graphData = useMemo(() => query.data ?? { nodes: [], links: [] }, [query.data]);

  return (
    <section className="page-grid graph-page">
      <div className="page-header">
        <div>
          <h1>Knowledge Graph</h1>
          <p>Inspect focused neighborhoods or the whole DB graph.</p>
        </div>
      </div>

      <section className="toolbar-band">
        <FormField label="Scope">
          <select value={scope} onChange={(event) => setScope(event.target.value as "whole" | "focused")}>
            <option value="whole">whole KB</option>
            <option value="focused">focused page</option>
          </select>
        </FormField>
        <FormField label="Slug">
          <input value={slug} onChange={(event) => setSlug(event.target.value)} disabled={scope === "whole"} placeholder="people/alice" />
        </FormField>
        <FormField label="Depth">
          <input type="number" min={0} max={4} value={depth} onChange={(event) => setDepth(Number(event.target.value))} />
        </FormField>
      </section>

      {query.error ? <div className="notice error">{query.error.message}</div> : null}

      <div className="graph-canvas">
        <ForceGraph2D
          graphData={graphData}
          width={Math.min(window.innerWidth - 360, 1100)}
          height={640}
          nodeLabel={(node: ForceNode) => `${node.title ?? node.label} (${node.type ?? "page"})`}
          linkLabel={(link: ForceLink) => link.label ?? "related"}
          nodeCanvasObject={(node: ForceNode, ctx: CanvasRenderingContext2D, globalScale: number) => {
            const label = String(node.label ?? node.id);
            const fontSize = Math.max(3.5, 11 / globalScale);
            ctx.font = `${fontSize}px Inter, sans-serif`;
            ctx.fillStyle = node.type === "person" ? "#0f766e" : node.type === "project" ? "#9a3412" : "#334155";
            ctx.beginPath();
            ctx.arc(Number(node.x ?? 0), Number(node.y ?? 0), 4.5, 0, 2 * Math.PI, false);
            ctx.fill();
            ctx.fillStyle = "#111827";
            ctx.fillText(label, Number(node.x ?? 0) + 7, Number(node.y ?? 0) + 4);
          }}
          linkDirectionalArrowLength={3.5}
          linkDirectionalArrowRelPos={1}
        />
      </div>
    </section>
  );
}
