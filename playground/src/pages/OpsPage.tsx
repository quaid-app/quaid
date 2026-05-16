import { useMutation, useQuery } from "@tanstack/react-query";
import { Play } from "lucide-react";
import { useMemo, useState } from "react";
import { FormField } from "../components/FormField";
import { JsonPanel } from "../components/JsonPanel";
import { apiGet, callMcp, runCli } from "../lib/api";
import type { CliCommandSpec } from "../lib/types";

export function OpsPage() {
  const { data: commands = [] } = useQuery<CliCommandSpec[]>({ queryKey: ["cli-catalog"], queryFn: () => apiGet("/cli/catalog") });
  const [tool, setTool] = useState("memory_stats");
  const [toolArgs, setToolArgs] = useState("{}");
  const [commandId, setCommandId] = useState("status.json");
  const [commandParams, setCommandParams] = useState("{}");

  const selectedCommand = useMemo(() => commands.find((command) => command.id === commandId), [commands, commandId]);

  const mcpMutation = useMutation({
    mutationFn: () => callMcp(tool, JSON.parse(toolArgs || "{}"))
  });

  const cliMutation = useMutation({
    mutationFn: () => {
      const params = JSON.parse(commandParams || "{}") as Record<string, unknown>;
      const confirmed = selectedCommand?.risky ? window.confirm(`Run ${selectedCommand.label}?`) : false;
      if (selectedCommand?.risky && !confirmed) {
        throw new Error("Command cancelled.");
      }
      return runCli(commandId, params, confirmed);
    }
  });

  return (
    <section className="page-grid">
      <div className="page-header">
        <div>
          <h1>Ops Console</h1>
          <p>Raw MCP and allowlisted CLI controls for advanced testing.</p>
        </div>
      </div>

      <section className="split-layout">
        <div className="tool-panel">
          <h2>MCP Tool</h2>
          <FormField label="Tool name">
            <input value={tool} onChange={(event) => setTool(event.target.value)} />
          </FormField>
          <FormField label="Arguments JSON">
            <textarea value={toolArgs} onChange={(event) => setToolArgs(event.target.value)} rows={8} />
          </FormField>
          <button className="primary-button" type="button" onClick={() => mcpMutation.mutate()} disabled={mcpMutation.isPending}>
            <Play size={16} />
            <span>Call Tool</span>
          </button>
        </div>
        <div className="tool-panel">
          <h2>CLI Bridge</h2>
          <FormField label="Command">
            <select value={commandId} onChange={(event) => setCommandId(event.target.value)}>
              {commands.map((command) => (
                <option key={command.id} value={command.id}>
                  {command.label}
                </option>
              ))}
            </select>
          </FormField>
          <p className="muted">{selectedCommand?.description}</p>
          <FormField label="Parameters JSON">
            <textarea value={commandParams} onChange={(event) => setCommandParams(event.target.value)} rows={8} />
          </FormField>
          <button className="primary-button" type="button" onClick={() => cliMutation.mutate()} disabled={cliMutation.isPending}>
            <Play size={16} />
            <span>Run Command</span>
          </button>
        </div>
      </section>

      <section className="split-layout">
        <JsonPanel title="MCP Result" value={mcpMutation.data ?? mcpMutation.error?.message ?? "No MCP call yet."} />
        <JsonPanel title="CLI Result" value={cliMutation.data ?? cliMutation.error?.message ?? "No CLI command yet."} />
      </section>
    </section>
  );
}
