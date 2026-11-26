// The one Server-Sent Events reader both stream kinds share: an agent
// invocation (`AgentStream`) and a route that returns `Stream<T>`
// (`RouteStream`). It splits the body into events and parses each
// event's data as JSON; mapping events to the typed unions is the
// caller's.

/** One event as it arrived: its name and its parsed `data`. */
export interface SseEvent {
  event: string;
  data: unknown;
}

/**
 * Read `body` event by event. The generator returns normally when the
 * body ends and throws when the connection fails mid-read, so a caller
 * can tell an ended stream from a lost one.
 */
export async function* readSseEvents(body: ReadableStream<Uint8Array>): AsyncGenerator<SseEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true }).replace(/\r\n?/g, "\n");
      let sep: number;
      while ((sep = buffer.indexOf("\n\n")) !== -1) {
        const raw = buffer.slice(0, sep);
        buffer = buffer.slice(sep + 2);
        const event = parseSseEvent(raw);
        if (event) yield event;
      }
    }
  } finally {
    reader.releaseLock();
  }
}

function parseSseEvent(raw: string): SseEvent | null {
  let event = "message";
  const dataLines: string[] = [];
  for (const line of raw.split("\n")) {
    if (line.startsWith("event:")) event = line.slice(6).trim();
    else if (line.startsWith("data:")) dataLines.push(line.slice(5).replace(/^ /, ""));
  }
  if (dataLines.length === 0) return null;
  const text = dataLines.join("\n");
  try {
    return { event, data: JSON.parse(text) };
  } catch {
    return { event, data: text };
  }
}
