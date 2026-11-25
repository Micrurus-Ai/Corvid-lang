import { CorvidError } from "./errors.js";
import { LiveRows, type LiveDelivery } from "./live-rows.js";

export interface LiveObserver<Row> {
  /** Receives the complete ordered result after each delivery. */
  rows(value: readonly Row[]): void;
  /** Terminal failure. Discard any previously displayed result. */
  error(error: Error): void;
}

interface Subscription {
  path: string;
  ready: boolean;
  apply(frame: LiveDelivery<object>): () => void;
  error(error: Error): void;
}

/** The RT-3 session shared by subscriptions on one browser client. */
export class LiveConnection {
  private readonly subscriptions = new Map<string, Subscription>();
  private readonly url: URL;
  private socket?: WebSocket;
  private retry?: ReturnType<typeof setTimeout>;
  private sequence = 0;

  constructor(baseUrl: string) {
    this.url = new URL("/.well-known/corvid/live", new URL(baseUrl || ".", globalThis.location?.href));
    if (!["http:", "https:"].includes(this.url.protocol)) throw new Error("Expected an HTTP base URL");
    this.url.protocol = this.url.protocol === "https:" ? "wss:" : "ws:";
  }

  subscribe<Row extends object>(
    path: string, keyFields: readonly (keyof Row)[], observer: LiveObserver<Row>,
  ): () => void {
    const rows = new LiveRows<Row>(keyFields);
    const id = String(++this.sequence);
    const sub: Subscription = {
      path, ready: false, error: error => observer.error(error),
      apply(frame) {
        const value = rows.apply(frame as LiveDelivery<Row>);
        return () => observer.rows(value);
      },
    };
    this.subscriptions.set(id, sub);
    try {
      if (!this.socket && !this.retry) this.connect();
      if (this.socket?.readyState === WebSocket.OPEN) this.sendSubscribe(id, sub);
    } catch (error) {
      this.subscriptions.delete(id);
      throw error;
    }
    return () => {
      if (!this.subscriptions.delete(id)) return;
      if (this.socket?.readyState === WebSocket.OPEN) {
        this.socket.send(JSON.stringify({ type: "unsubscribe", id }));
      }
      this.closeIfIdle();
    };
  }

  private sendSubscribe(id: string, sub: Subscription): void {
    sub.ready = false;
    this.socket!.send(JSON.stringify({ type: "subscribe", id, path: sub.path }));
  }

  private closeIfIdle(): void {
    if (this.subscriptions.size) return;
    clearTimeout(this.retry);
    this.retry = undefined;
    const socket = this.socket;
    this.socket = undefined;
    socket?.close(1000);
  }

  private connect(): void {
    const socket = new WebSocket(this.url, "corvid.live.v1");
    this.socket = socket;
    socket.onopen = () => {
      if (this.socket !== socket) return;
      for (const [id, sub] of this.subscriptions) this.sendSubscribe(id, sub);
    };
    socket.onmessage = event => {
      if (this.socket !== socket) return;
      let notify: () => void;
      try {
        const frame = JSON.parse(event.data);
        if (typeof frame?.id !== "string") throw new Error("Missing live subscription id");
        const sub = this.subscriptions.get(frame.id);
        if (!sub) return; // Delivery already in flight when its owner unsubscribed.
        if (frame.type === "error" && frame.code === "forbidden") {
          this.subscriptions.delete(frame.id);
          this.closeIfIdle();
          notify = () => sub.error(new CorvidError(403, { code: "forbidden" }));
        } else {
          if (frame.type !== "snapshot" && !(sub.ready && frame.type === "change" &&
              ["added", "updated", "removed"].includes(frame.kind))) {
            throw new Error("Expected a live snapshot or ordered change");
          }
          notify = sub.apply(frame);
          sub.ready = true;
        }
      } catch (error) {
        this.socket = undefined;
        // Browser scripts may send 1000 or application close codes only.
        socket.close(4002, "Invalid live delivery");
        this.failAll(error instanceof Error ? error : new Error("Invalid live delivery"));
        return;
      }
      notify();
    };
    socket.onclose = event => {
      if (this.socket !== socket) return;
      this.socket = undefined;
      if (event.code === 1008 || event.code === 1002) {
        this.failAll(event.code === 1008
          ? new CorvidError(403, { code: "invalid_session" })
          : new Error("Live protocol rejected"));
      } else if (this.subscriptions.size) {
        this.retry = setTimeout(() => {
          this.retry = undefined;
          try { this.connect(); }
          catch (error) { this.failAll(error instanceof Error ? error : new Error("Live reconnect failed")); }
        }, 500);
      }
    };
  }

  private failAll(error: Error): void {
    const subscriptions = [...this.subscriptions.values()];
    this.subscriptions.clear();
    for (const sub of subscriptions) queueMicrotask(() => sub.error(error));
  }
}
