import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Play, RefreshCcw, Square } from "lucide-react";
import { apiGet, apiPost } from "../lib/api";
import type { ApiStatus } from "../lib/types";

export function StatusPanel() {
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery<ApiStatus>({
    queryKey: ["status"],
    queryFn: () => apiGet<ApiStatus>("/status"),
    refetchInterval: 10_000
  });

  const runtimeMutation = useMutation({
    mutationFn: (action: "start" | "stop" | "restart") => apiPost(`/runtime/${action}`),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["status"] })
  });

  if (isLoading) {
    return <section className="status-panel">Loading status...</section>;
  }

  if (error || !data) {
    return <section className="status-panel error">Status unavailable</section>;
  }

  const runtime = data.runtime;
  const database = data.database;

  return (
    <section className="status-panel" aria-label="Runtime status">
      <div className="status-row">
        <span>Runtime</span>
        <strong>{runtime.mode}</strong>
      </div>
      <div className="status-row">
        <span>MCP</span>
        <strong>{runtime.managedRunning || runtime.mode === "external" ? "ready" : "stopped"}</strong>
      </div>
      <div className="status-row">
        <span>DB</span>
        <strong>{database.databaseExists ? `${database.pageCount} pages` : "missing"}</strong>
      </div>
      <div className="status-row">
        <span>Embedding</span>
        <strong>{database.activeEmbedding?.name ?? data.config.embeddingModel}</strong>
      </div>
      <div className="status-row">
        <span>Extraction</span>
        <strong>{database.config["extraction.enabled"] ?? "unknown"}</strong>
      </div>
      <div className="runtime-actions">
        <button
          className="icon-button"
          type="button"
          title="Start managed runtime"
          disabled={runtime.mode === "external" || runtimeMutation.isPending}
          onClick={() => runtimeMutation.mutate("start")}
        >
          <Play size={16} />
        </button>
        <button
          className="icon-button"
          type="button"
          title="Restart managed runtime"
          disabled={runtime.mode === "external" || runtimeMutation.isPending}
          onClick={() => runtimeMutation.mutate("restart")}
        >
          <RefreshCcw size={16} />
        </button>
        <button
          className="icon-button"
          type="button"
          title="Stop managed runtime"
          disabled={runtime.mode === "external" || runtimeMutation.isPending}
          onClick={() => runtimeMutation.mutate("stop")}
        >
          <Square size={16} />
        </button>
      </div>
      <div className="path-readout" title={data.config.dbPath}>
        {data.config.dbPath}
      </div>
      <details className="runtime-log">
        <summary>Runtime log</summary>
        <pre>{runtime.logTail.length ? runtime.logTail.join("\n") : "No Quaid runtime output yet."}</pre>
      </details>
    </section>
  );
}
