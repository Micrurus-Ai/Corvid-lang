import { CorvidError } from "./errors.js";
import { LiveRows } from "./live-rows.js";
// Independently loaded generated clients and Corvid views on the same page
// subscribe under the same browser session. Other origins stay isolated.
const pageLive = Symbol.for("corvid.live.v1.page");
export function connectionForPage(baseUrl) {
    if (typeof window === "undefined" || new URL(baseUrl || ".", location.href).origin !== location.origin) {
        return new LiveConnection(baseUrl);
    }
    const page = window;
    return page[pageLive] ??= new LiveConnection(location.origin);
}
/** The RT-3 session shared by subscriptions on one browser client. */
export class LiveConnection {
    subscriptions = new Map();
    url;
    socket;
    retry;
    sequence = 0;
    constructor(baseUrl) {
        this.url = new URL("/.well-known/corvid/live", new URL(baseUrl || ".", globalThis.location?.href));
        if (!["http:", "https:"].includes(this.url.protocol))
            throw new Error("Expected an HTTP base URL");
        this.url.protocol = this.url.protocol === "https:" ? "wss:" : "ws:";
    }
    subscribe(path, keyFields, observer,
    // Each SDK bundle delivers instances of its own exported error class.
    ErrorType = CorvidError) {
        const rows = new LiveRows(keyFields);
        const id = String(++this.sequence);
        let active = true;
        const sub = {
            path, ready: false,
            error: failure => {
                if (active)
                    observer.error(typeof failure === "string" ? new ErrorType(403, { code: failure }) : failure);
            },
            apply(frame) {
                const value = rows.apply(frame);
                return () => observer.rows(value);
            },
        };
        this.subscriptions.set(id, sub);
        try {
            if (!this.socket && !this.retry)
                this.connect();
            if (this.socket?.readyState === WebSocket.OPEN)
                this.sendSubscribe(id, sub);
        }
        catch (error) {
            this.subscriptions.delete(id);
            throw error;
        }
        return () => {
            active = false; // A terminal error may already be queued for this observer.
            if (!this.subscriptions.delete(id))
                return;
            if (this.socket?.readyState === WebSocket.OPEN) {
                this.socket.send(JSON.stringify({ type: "unsubscribe", id }));
            }
            this.closeIfIdle();
        };
    }
    sendSubscribe(id, sub) {
        sub.ready = false;
        this.socket.send(JSON.stringify({ type: "subscribe", id, path: sub.path }));
    }
    closeIfIdle() {
        if (this.subscriptions.size)
            return;
        clearTimeout(this.retry);
        this.retry = undefined;
        const socket = this.socket;
        this.socket = undefined;
        socket?.close(1000);
    }
    connect() {
        const socket = new WebSocket(this.url, "corvid.live.v1");
        this.socket = socket;
        socket.onopen = () => {
            if (this.socket !== socket)
                return;
            for (const [id, sub] of this.subscriptions)
                this.sendSubscribe(id, sub);
        };
        socket.onmessage = event => {
            if (this.socket !== socket)
                return;
            let notify;
            try {
                const frame = JSON.parse(event.data);
                if (typeof frame?.id !== "string")
                    throw new Error("Missing live subscription id");
                const sub = this.subscriptions.get(frame.id);
                if (!sub)
                    return; // Delivery already in flight when its owner unsubscribed.
                if (frame.type === "error" && frame.code === "forbidden") {
                    this.subscriptions.delete(frame.id);
                    this.closeIfIdle();
                    notify = () => sub.error("forbidden");
                }
                else {
                    if (frame.type !== "snapshot" && !(sub.ready && frame.type === "change" &&
                        ["added", "updated", "removed"].includes(frame.kind))) {
                        throw new Error("Expected a live snapshot or ordered change");
                    }
                    notify = sub.apply(frame);
                    sub.ready = true;
                }
            }
            catch (error) {
                this.socket = undefined;
                // Browser scripts may send 1000 or application close codes only.
                socket.close(4002, "Invalid live delivery");
                this.failAll(error instanceof Error ? error : new Error("Invalid live delivery"));
                return;
            }
            notify();
        };
        socket.onclose = event => {
            if (this.socket !== socket)
                return;
            this.socket = undefined;
            if (event.code === 1008 || event.code === 1002) {
                this.failAll(event.code === 1008
                    ? "invalid_session"
                    : new Error("Live protocol rejected"));
            }
            else if (this.subscriptions.size) {
                this.retry = setTimeout(() => {
                    this.retry = undefined;
                    try {
                        this.connect();
                    }
                    catch (error) {
                        this.failAll(error instanceof Error ? error : new Error("Live reconnect failed"));
                    }
                }, 500);
            }
        };
    }
    failAll(error) {
        const subscriptions = [...this.subscriptions.values()];
        this.subscriptions.clear();
        for (const sub of subscriptions)
            queueMicrotask(() => sub.error(error));
    }
}
