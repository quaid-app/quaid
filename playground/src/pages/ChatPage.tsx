import { useMutation } from "@tanstack/react-query";
import { Send } from "lucide-react";
import { useMemo, useState } from "react";
import { FormField } from "../components/FormField";
import { ResultCard } from "../components/ResultCard";
import { composeRetrievalAnswer, queryAndEnrich } from "../lib/search";
import type { EnrichedResult } from "../lib/types";

export function ChatPage() {
  const [question, setQuestion] = useState("");
  const [namespace, setNamespace] = useState("");
  const [limit, setLimit] = useState(8);
  const [lastQuestion, setLastQuestion] = useState("");

  const mutation = useMutation({
    mutationFn: () => queryAndEnrich(question, limit, namespace),
    onSuccess: () => setLastQuestion(question)
  });

  const results = mutation.data ?? [];
  const answer = useMemo(() => composeRetrievalAnswer(lastQuestion, results), [lastQuestion, results]);

  function submit() {
    if (!question.trim()) {
      return;
    }
    mutation.mutate();
  }

  return (
    <section className="page-grid chat-page">
      <div className="page-header">
        <div>
          <h1>Memory Search</h1>
          <p>Ask against Quaid memory and inspect the retrieved evidence.</p>
        </div>
      </div>

      <section className="chat-surface">
        <div className="question-box">
          <textarea
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder="What is my preferred beverage?"
            rows={4}
            onKeyDown={(event) => {
              if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                submit();
              }
            }}
          />
          <div className="question-controls">
            <FormField label="Namespace">
              <input value={namespace} onChange={(event) => setNamespace(event.target.value)} placeholder="optional" />
            </FormField>
            <FormField label="Limit">
              <input
                type="number"
                min={1}
                max={30}
                value={limit}
                onChange={(event) => setLimit(Number(event.target.value))}
              />
            </FormField>
            <button className="primary-button" type="button" onClick={submit} disabled={mutation.isPending}>
              <Send size={16} />
              <span>{mutation.isPending ? "Searching" : "Ask"}</span>
            </button>
          </div>
        </div>

        {mutation.error ? <div className="notice error">{mutation.error.message}</div> : null}

        {mutation.data ? (
          <div className="answer-band">
            <h2>Retrieval Answer</h2>
            <p>{answer}</p>
          </div>
        ) : null}
      </section>

      <ResultList results={results} />
    </section>
  );
}

function ResultList({ results }: { results: EnrichedResult[] }) {
  if (results.length === 0) {
    return <div className="empty-state">No results yet.</div>;
  }
  return (
    <section className="result-grid" aria-label="Search results">
      {results.map((item) => (
        <ResultCard key={item.result.slug} item={item} />
      ))}
    </section>
  );
}
