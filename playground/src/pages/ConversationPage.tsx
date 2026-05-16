import { useMutation } from "@tanstack/react-query";
import { CirclePlus, ScanSearch, Send, SquareCheckBig } from "lucide-react";
import { useMemo, useState } from "react";
import { FormField } from "../components/FormField";
import { JsonPanel } from "../components/JsonPanel";
import { ResultCard } from "../components/ResultCard";
import { callMcp, runCli } from "../lib/api";
import { composeRetrievalAnswer, queryAndEnrich } from "../lib/search";
import type { EnrichedResult } from "../lib/types";

function defaultSessionId() {
  return `playground-${new Date().toISOString().replace(/[:.]/g, "-")}`;
}

export function ConversationPage() {
  const [sessionId, setSessionId] = useState(defaultSessionId);
  const [namespace, setNamespace] = useState("");
  const [role, setRole] = useState("user");
  const [content, setContent] = useState("I like to drink coffee more than tea.");
  const [question, setQuestion] = useState("What is my preferred beverage?");
  const [results, setResults] = useState<EnrichedResult[]>([]);
  const [lastQuestion, setLastQuestion] = useState(question);

  const addTurn = useMutation({
    mutationFn: () =>
      callMcp("memory_add_turn", {
        session_id: sessionId,
        role,
        content,
        namespace: namespace || undefined,
        metadata: {
          source: "quaid-playground"
        }
      })
  });

  const closeSession = useMutation({
    mutationFn: () =>
      callMcp("memory_close_session", {
        session_id: sessionId,
        namespace: namespace || undefined
      })
  });

  const forceExtract = useMutation({
    mutationFn: () => runCli("extract.session", { sessionId, force: true }, true)
  });

  const ask = useMutation({
    mutationFn: async () => {
      const items = await queryAndEnrich(question, 10, namespace);
      setResults(items);
      setLastQuestion(question);
      return items;
    }
  });

  const answer = useMemo(() => composeRetrievalAnswer(lastQuestion, results), [lastQuestion, results]);

  return (
    <section className="page-grid">
      <div className="page-header">
        <div>
          <h1>Conversation Lab</h1>
          <p>Store turns, trigger extraction, then verify facts through retrieval.</p>
        </div>
      </div>

      <section className="split-layout">
        <div className="tool-panel">
          <div className="two-col">
            <FormField label="Session ID">
              <input value={sessionId} onChange={(event) => setSessionId(event.target.value)} />
            </FormField>
            <FormField label="Namespace">
              <input value={namespace} onChange={(event) => setNamespace(event.target.value)} placeholder="optional" />
            </FormField>
          </div>
          <FormField label="Role">
            <select value={role} onChange={(event) => setRole(event.target.value)}>
              <option value="user">user</option>
              <option value="assistant">assistant</option>
              <option value="system">system</option>
            </select>
          </FormField>
          <FormField label="Turn content">
            <textarea value={content} onChange={(event) => setContent(event.target.value)} rows={6} />
          </FormField>
          <div className="button-row">
            <button className="primary-button" type="button" onClick={() => addTurn.mutate()} disabled={addTurn.isPending}>
              <CirclePlus size={16} />
              <span>Add Turn</span>
            </button>
            <button className="secondary-button" type="button" onClick={() => closeSession.mutate()} disabled={closeSession.isPending}>
              <SquareCheckBig size={16} />
              <span>Close</span>
            </button>
            <button className="secondary-button" type="button" onClick={() => forceExtract.mutate()} disabled={forceExtract.isPending}>
              <ScanSearch size={16} />
              <span>Extract</span>
            </button>
          </div>
          <FormField label="Question">
            <input value={question} onChange={(event) => setQuestion(event.target.value)} />
          </FormField>
          <button className="primary-button" type="button" onClick={() => ask.mutate()} disabled={ask.isPending}>
            <Send size={16} />
            <span>Ask Memory</span>
          </button>
        </div>

        <div className="stack">
          <JsonPanel title="Add Turn Result" value={addTurn.data ?? addTurn.error?.message ?? "No turn added yet."} />
          <JsonPanel title="Close / Extract Result" value={closeSession.data ?? forceExtract.data ?? closeSession.error?.message ?? forceExtract.error?.message ?? "No extraction action yet."} />
        </div>
      </section>

      {ask.data ? (
        <section className="answer-band">
          <h2>Fact Check</h2>
          <p>{answer}</p>
        </section>
      ) : null}
      <section className="result-grid">
        {results.map((item) => (
          <ResultCard key={item.result.slug} item={item} />
        ))}
      </section>
    </section>
  );
}
