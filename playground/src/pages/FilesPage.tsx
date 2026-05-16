import { useQuery } from "@tanstack/react-query";
import { useState } from "react";
import { FormField } from "../components/FormField";
import { JsonPanel } from "../components/JsonPanel";
import { MarkdownView } from "../components/MarkdownView";
import { TreeView } from "../components/TreeView";
import { apiGet } from "../lib/api";
import type { ApiStatus, TreeNode } from "../lib/types";

type ViewerState = {
  title: string;
  markdown?: string;
  raw?: unknown;
};

export function FilesPage() {
  const { data: status } = useQuery<ApiStatus>({ queryKey: ["status"], queryFn: () => apiGet("/status") });
  const { data: dbTree = [] } = useQuery<TreeNode[]>({ queryKey: ["db-tree"], queryFn: () => apiGet("/db/tree") });
  const [root, setRoot] = useState("");
  const liveTree = useQuery<TreeNode>({
    queryKey: ["live-tree", root],
    queryFn: () => apiGet(`/files/tree?root=${encodeURIComponent(root)}`),
    enabled: Boolean(root)
  });
  const [viewer, setViewer] = useState<ViewerState>({ title: "Viewer", markdown: "" });

  async function selectNode(node: TreeNode) {
    if (node.type === "page") {
      const slug = String(node.meta?.collection_name ? `${node.meta.collection_name}::${node.meta.slug}` : node.meta?.slug ?? node.label);
      const page = await apiGet<{ markdown: string; title: string }>(`/db/page?slug=${encodeURIComponent(slug)}`);
      setViewer({ title: page.title, markdown: page.markdown, raw: page });
      return;
    }
    if (node.type === "raw-import") {
      const rawId = Number(node.meta?.id);
      const raw = await apiGet<{ content: string; filePath: string }>(`/db/raw-file?id=${rawId}`);
      setViewer({ title: raw.filePath, markdown: raw.content, raw });
      return;
    }
    if (node.type === "file" && node.path) {
      const file = await apiGet<{ content: string; path: string }>(`/files/read?path=${encodeURIComponent(node.path)}`);
      setViewer({ title: file.path, markdown: file.content, raw: file });
    }
  }

  return (
    <section className="page-grid">
      <div className="page-header">
        <div>
          <h1>Files</h1>
          <p>Browse DB-backed raw imports, pages, and live collection files.</p>
        </div>
      </div>

      <section className="file-layout">
        <div className="browser-pane">
          <h2>Database</h2>
          <TreeView nodes={dbTree} onSelect={selectNode} />
        </div>
        <div className="browser-pane">
          <h2>Live Collection</h2>
          <FormField label="Root">
            <select value={root} onChange={(event) => setRoot(event.target.value)}>
              <option value="">Select root</option>
              {status?.database.collections.map((collection) => (
                <option key={collection.id} value={collection.root_path}>
                  {collection.name}
                </option>
              ))}
            </select>
          </FormField>
          {liveTree.data ? <TreeView nodes={[liveTree.data]} onSelect={selectNode} /> : <div className="empty-state">Select a collection root.</div>}
        </div>
        <div className="viewer-pane">
          <h2>{viewer.title}</h2>
          <MarkdownView markdown={viewer.markdown ?? ""} />
          <JsonPanel title="Metadata" value={viewer.raw ?? "No file selected."} />
        </div>
      </section>
    </section>
  );
}
