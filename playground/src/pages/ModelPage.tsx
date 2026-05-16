import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Download, Play, RefreshCcw, SlidersHorizontal } from "lucide-react";
import { useState } from "react";
import { FormField } from "../components/FormField";
import { JsonPanel } from "../components/JsonPanel";
import { apiGet, apiPost, runCli } from "../lib/api";
import type { ApiStatus, EmbeddingProfile } from "../lib/types";

const slmPresets = ["phi-3.5-mini", "gemma-3-1b", "gemma-3-4b", "custom"];
const embeddingPresets = ["small", "base", "large", "m3", "custom"];

export function ModelPage() {
  const queryClient = useQueryClient();
  const { data: status } = useQuery<ApiStatus>({ queryKey: ["status"], queryFn: () => apiGet("/status") });
  const { data: profiles = [] } = useQuery<EmbeddingProfile[]>({ queryKey: ["profiles"], queryFn: () => apiGet("/profiles") });
  const [slmPreset, setSlmPreset] = useState("phi-3.5-mini");
  const [customSlm, setCustomSlm] = useState("");
  const [embeddingPreset, setEmbeddingPreset] = useState("small");
  const [customEmbedding, setCustomEmbedding] = useState("");
  const [profileName, setProfileName] = useState("small-test");
  const [selectedProfile, setSelectedProfile] = useState("");
  const [compareQuery, setCompareQuery] = useState("What is my preferred beverage?");

  const selectedSlm = slmPreset === "custom" ? customSlm : slmPreset;
  const selectedEmbedding = embeddingPreset === "custom" ? customEmbedding : embeddingPreset;

  const cliMutation = useMutation({
    mutationFn: ({ id, params }: { id: string; params?: Record<string, unknown> }) => runCli(id, params ?? {}, true),
    onSettled: () => queryClient.invalidateQueries({ queryKey: ["status"] })
  });

  const createProfile = useMutation({
    mutationFn: () => apiPost<EmbeddingProfile>("/profiles/create", { name: profileName, modelAlias: selectedEmbedding }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["profiles"] })
  });

  const embedProfile = useMutation({
    mutationFn: () => apiPost("/profiles/embed", { name: selectedProfile }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["profiles"] })
  });

  const compare = useMutation({
    mutationFn: async () => {
      const targets = profiles.length ? profiles : [];
      return Promise.all(
        targets.map((profile) => apiPost("/profiles/query", { name: profile.name, query: compareQuery, limit: 8 }))
      );
    }
  });

  return (
    <section className="page-grid">
      <div className="page-header">
        <div>
          <h1>Model Lab</h1>
          <p>Switch extraction SLMs and compare embedding-model profile databases.</p>
        </div>
      </div>

      <section className="split-layout">
        <div className="tool-panel">
          <h2>Fact Extraction SLM</h2>
          <div className="two-col">
            <FormField label="Preset">
              <select value={slmPreset} onChange={(event) => setSlmPreset(event.target.value)}>
                {slmPresets.map((preset) => (
                  <option key={preset} value={preset}>
                    {preset}
                  </option>
                ))}
              </select>
            </FormField>
            <FormField label="Custom alias">
              <input value={customSlm} onChange={(event) => setCustomSlm(event.target.value)} disabled={slmPreset !== "custom"} />
            </FormField>
          </div>
          <div className="button-row">
            <button className="primary-button" type="button" onClick={() => cliMutation.mutate({ id: "model.pull", params: { alias: selectedSlm } })}>
              <Download size={16} />
              <span>Download</span>
            </button>
            <button className="secondary-button" type="button" onClick={() => cliMutation.mutate({ id: "config.set", params: { key: "extraction.model_alias", value: selectedSlm } })}>
              <SlidersHorizontal size={16} />
              <span>Select</span>
            </button>
            <button className="secondary-button" type="button" onClick={() => cliMutation.mutate({ id: "extraction.enable" })}>
              <Play size={16} />
              <span>Enable</span>
            </button>
            <button className="secondary-button" type="button" onClick={() => apiPost("/runtime/restart").then(() => queryClient.invalidateQueries({ queryKey: ["status"] }))}>
              <RefreshCcw size={16} />
              <span>Restart</span>
            </button>
          </div>
          <dl className="metadata-grid wide">
            <div>
              <dt>Configured</dt>
              <dd>{status?.database.config["extraction.model_alias"] ?? "-"}</dd>
            </div>
            <div>
              <dt>Enabled</dt>
              <dd>{status?.database.config["extraction.enabled"] ?? "-"}</dd>
            </div>
            <div>
              <dt>Queue</dt>
              <dd>{status?.database.extractionQueue ?? 0}</dd>
            </div>
            <div>
              <dt>Runtime</dt>
              <dd>{status?.runtime.mode ?? "-"}</dd>
            </div>
          </dl>
        </div>

        <div className="tool-panel">
          <h2>Embedding Profiles</h2>
          <div className="two-col">
            <FormField label="Profile name">
              <input value={profileName} onChange={(event) => setProfileName(event.target.value)} />
            </FormField>
            <FormField label="Embedding model">
              <select value={embeddingPreset} onChange={(event) => setEmbeddingPreset(event.target.value)}>
                {embeddingPresets.map((preset) => (
                  <option key={preset} value={preset}>
                    {preset}
                  </option>
                ))}
              </select>
            </FormField>
          </div>
          {embeddingPreset === "custom" ? (
            <FormField label="Custom embedding ID">
              <input value={customEmbedding} onChange={(event) => setCustomEmbedding(event.target.value)} />
            </FormField>
          ) : null}
          <div className="button-row">
            <button className="primary-button" type="button" onClick={() => createProfile.mutate()} disabled={createProfile.isPending}>
              Create Profile
            </button>
            <select value={selectedProfile} onChange={(event) => setSelectedProfile(event.target.value)}>
              <option value="">Select profile</option>
              {profiles.map((profile) => (
                <option key={profile.name} value={profile.name}>
                  {profile.name} ({profile.modelAlias})
                </option>
              ))}
            </select>
            <button className="secondary-button" type="button" onClick={() => embedProfile.mutate()} disabled={!selectedProfile || embedProfile.isPending}>
              Embed All
            </button>
          </div>
          <FormField label="Compare query">
            <input value={compareQuery} onChange={(event) => setCompareQuery(event.target.value)} />
          </FormField>
          <button className="secondary-button" type="button" onClick={() => compare.mutate()} disabled={!profiles.length || compare.isPending}>
            Compare Profiles
          </button>
        </div>
      </section>

      <section className="split-layout">
        <JsonPanel title="Last model command" value={cliMutation.data ?? cliMutation.error?.message ?? "No command run yet."} />
        <JsonPanel title="Profile output" value={createProfile.data ?? embedProfile.data ?? compare.data ?? createProfile.error?.message ?? embedProfile.error?.message ?? compare.error?.message ?? profiles} />
      </section>
    </section>
  );
}
