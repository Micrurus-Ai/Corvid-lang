// Streaming agent view (slice 51p). Renders the accumulated chunks and
// the typed event log from `useCorvidStream`.

import { type ReactNode } from "react";
import { type AgentStream } from "@corvid/client";
import { useCorvidStream } from "../useCorvidAgent.js";

export interface CorvidStreamProps<Args extends unknown[], T> {
  /** The streaming call, e.g. `(m) => api.chat(m)`. */
  stream: (...args: Args) => AgentStream<T>;
  /** Arguments to start the stream with (rendered with a Start button). */
  args: Args;
  /** Render each chunk. Defaults to `String(chunk)`. */
  renderChunk?: (chunk: T, index: number) => ReactNode;
  className?: string;
}

/** Start a streaming agent and render its chunks + a live event log. */
export function CorvidStream<Args extends unknown[], T>(
  props: CorvidStreamProps<Args, T>,
): ReactNode {
  const { stream, args, renderChunk, className } = props;
  const s = useCorvidStream(stream);

  return (
    <div className={className}>
      <button onClick={() => void s.start(...args)} disabled={s.streaming}>
        {s.streaming ? "streaming…" : "Start"}
      </button>
      <div className="corvid-chunks">
        {s.chunks.map((c, i) => (renderChunk ? renderChunk(c, i) : <span key={i}>{String(c)}</span>))}
      </div>
      <ol className="corvid-events" style={{ fontSize: 12, opacity: 0.7 }}>
        {s.events.map((e, i) => (
          <li key={i} data-kind={e.kind}>
            {e.kind}
          </li>
        ))}
      </ol>
      {s.error != null && <pre className="corvid-error">{String(s.error)}</pre>}
    </div>
  );
}
