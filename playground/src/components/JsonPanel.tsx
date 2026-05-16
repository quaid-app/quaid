import { prettyJson } from "../lib/api";

export function JsonPanel({ value, title = "Output" }: { value: unknown; title?: string }) {
  return (
    <section className="json-panel">
      <header>{title}</header>
      <pre>{prettyJson(value)}</pre>
    </section>
  );
}
