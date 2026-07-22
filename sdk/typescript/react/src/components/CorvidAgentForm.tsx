// Headless-ish prototype components (slice 51p).
//
// These are SCAFFOLDS — admin panels, internal tools, quick demos — not
// a claim to polished product UI. They accept `className` so a design
// system can restyle them, and they render nothing bespoke beyond the
// contract's shape. Build product UI with the hooks directly; reach for
// these to stand something up fast.

import { useState, type ReactNode } from "react";
import { CorvidError } from "@corvid/client";
import { useCorvidAgent } from "../useCorvidAgent.js";

export interface FieldSpec {
  name: string;
  label?: string;
  type?: "text" | "number";
  placeholder?: string;
}

export interface CorvidAgentFormProps<T> {
  /** Invoke the agent with the collected field values. */
  call: (values: Record<string, string>) => Promise<T>;
  /** The input fields to render. */
  fields: FieldSpec[];
  /** Render the successful result. Defaults to pretty-printed JSON. */
  renderResult?: (data: T) => ReactNode;
  /** Render a typed error. Defaults to the status + body. */
  renderError?: (error: CorvidError) => ReactNode;
  submitLabel?: string;
  className?: string;
}

/** A form that collects typed inputs and invokes an agent. */
export function CorvidAgentForm<T>(props: CorvidAgentFormProps<T>): ReactNode {
  const { call, fields, renderResult, renderError, submitLabel = "Run", className } = props;
  const [values, setValues] = useState<Record<string, string>>({});
  const agent = useCorvidAgent(call);

  return (
    <form
      className={className}
      onSubmit={(e) => {
        e.preventDefault();
        void agent.run(values).catch(() => {});
      }}
    >
      {fields.map((f) => (
        <label key={f.name} style={{ display: "block", margin: "6px 0" }}>
          <span style={{ display: "block", fontSize: 12, opacity: 0.7 }}>{f.label ?? f.name}</span>
          <input
            type={f.type ?? "text"}
            placeholder={f.placeholder}
            value={values[f.name] ?? ""}
            onChange={(e) => setValues((v) => ({ ...v, [f.name]: e.target.value }))}
          />
        </label>
      ))}
      <button type="submit" disabled={agent.loading}>
        {agent.loading ? "…" : submitLabel}
      </button>
      {agent.error &&
        (renderError ? (
          renderError(agent.error)
        ) : (
          <pre className="corvid-error">
            {agent.error.status}: {JSON.stringify(agent.error.body, null, 2)}
          </pre>
        ))}
      {agent.data != null &&
        (renderResult ? renderResult(agent.data) : <pre>{JSON.stringify(agent.data, null, 2)}</pre>)}
    </form>
  );
}
