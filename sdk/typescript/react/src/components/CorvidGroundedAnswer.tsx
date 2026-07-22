// Grounded-answer view (slice 51p). Renders a `Grounded<T>` value — the
// answer plus its provenance sources — matching the shape the TS
// generator emits for a grounded output (slice 51l):
//   { value: T; sources: Array<{ kind: string; name: string }> }.

import { type ReactNode } from "react";

export interface GroundedValue<T> {
  value: T;
  sources?: Array<{ kind: string; name: string }>;
}

export interface CorvidGroundedAnswerProps<T> {
  answer: GroundedValue<T>;
  /** Render the value. Defaults to `String(value)`. */
  renderValue?: (value: T) => ReactNode;
  className?: string;
}

/** Render a grounded answer with its citation/source list. */
export function CorvidGroundedAnswer<T>(props: CorvidGroundedAnswerProps<T>): ReactNode {
  const { answer, renderValue, className } = props;
  const sources = answer.sources ?? [];
  return (
    <div className={className}>
      <div className="corvid-answer">
        {renderValue ? renderValue(answer.value) : String(answer.value)}
      </div>
      {sources.length > 0 && (
        <ul className="corvid-sources" style={{ fontSize: 12, opacity: 0.7 }}>
          {sources.map((s, i) => (
            <li key={i}>
              <span data-kind={s.kind}>{s.kind}</span>: {s.name}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
