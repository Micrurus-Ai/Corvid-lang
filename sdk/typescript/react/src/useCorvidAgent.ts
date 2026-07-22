// React hooks for invoking Corvid agents (slice 51n).
//
// These are GENERIC over the call shape — a frontend passes a bound
// method from the generated `Api` (slice 51l) and the generated types
// specialize the hook automatically. Nothing here is per-app.

import { useCallback, useRef, useState } from "react";
import { CorvidError, type AgentEvent, type AgentStream } from "@corvid/client";

export interface AgentState<T> {
  /** The most recent result, or null before the first success. */
  data: T | null;
  /** The most recent error, or null. */
  error: CorvidError | null;
  /** Whether a call is in flight. */
  loading: boolean;
}

export interface AgentHandle<Args extends unknown[], T> extends AgentState<T> {
  /** Invoke the agent; resolves with the result and updates state. */
  run: (...args: Args) => Promise<T>;
  /** Clear the current data/error. */
  reset: () => void;
}

/**
 * Invoke a non-streaming agent/prompt. Pass a bound method from the
 * generated `Api`, e.g. `useCorvidAgent((q: string) => api.classify(q))`.
 */
export function useCorvidAgent<Args extends unknown[], T>(
  call: (...args: Args) => Promise<T>,
): AgentHandle<Args, T> {
  const [state, setState] = useState<AgentState<T>>({ data: null, error: null, loading: false });

  const run = useCallback(
    async (...args: Args): Promise<T> => {
      setState((s) => ({ ...s, loading: true, error: null }));
      try {
        const data = await call(...args);
        setState({ data, error: null, loading: false });
        return data;
      } catch (err) {
        const error = err instanceof CorvidError ? err : new CorvidError(0, err, String(err));
        setState((s) => ({ ...s, error, loading: false }));
        throw error;
      }
    },
    [call],
  );

  const reset = useCallback(() => setState({ data: null, error: null, loading: false }), []);
  return { ...state, run, reset };
}

export interface StreamState<T> {
  /** Every event observed so far, in order. */
  events: AgentEvent<T>[];
  /** Just the streamed `chunk` values, for incremental rendering. */
  chunks: T[];
  /** The terminal value once `completed` arrives, else null. */
  result: T | null;
  /** The terminal error once `failed` arrives, else null. */
  error: unknown | null;
  /** Whether the stream is open. */
  streaming: boolean;
}

export interface StreamHandle<Args extends unknown[], T> extends StreamState<T> {
  /** Open the stream; events accumulate into state as they arrive. */
  start: (...args: Args) => Promise<void>;
  reset: () => void;
}

/**
 * Consume a streaming agent's typed event stream. Pass a bound method
 * returning an `AgentStream<T>`, e.g.
 * `useCorvidStream((m: string) => api.chat(m))`.
 */
export function useCorvidStream<Args extends unknown[], T>(
  stream: (...args: Args) => AgentStream<T>,
): StreamHandle<Args, T> {
  const [state, setState] = useState<StreamState<T>>({
    events: [],
    chunks: [],
    result: null,
    error: null,
    streaming: false,
  });
  const active = useRef(false);

  const reset = useCallback(() => {
    active.current = false;
    setState({ events: [], chunks: [], result: null, error: null, streaming: false });
  }, []);

  const start = useCallback(
    async (...args: Args): Promise<void> => {
      active.current = true;
      setState({ events: [], chunks: [], result: null, error: null, streaming: true });
      try {
        for await (const event of stream(...args)) {
          if (!active.current) break;
          setState((s) => {
            const events = [...s.events, event];
            const chunks = event.kind === "chunk" ? [...s.chunks, event.value] : s.chunks;
            const result = event.kind === "completed" ? event.value : s.result;
            const error = event.kind === "failed" ? event.error : s.error;
            const streaming = event.kind !== "completed" && event.kind !== "failed";
            return { events, chunks, result, error, streaming };
          });
        }
      } catch (err) {
        setState((s) => ({ ...s, error: err, streaming: false }));
      } finally {
        active.current = false;
        setState((s) => ({ ...s, streaming: false }));
      }
    },
    [stream],
  );

  return { ...state, start, reset };
}
