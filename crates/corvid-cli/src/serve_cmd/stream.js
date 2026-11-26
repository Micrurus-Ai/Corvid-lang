// The server sends each item as a `chunk` event, then `completed` (data
// `null`) or `failed` (data `{error, detail}`). A connection that drops
// before either terminal event is reported as interrupted.
// Reconnect only on user request: a new GET starts a new model invocation.
class CorvidTextStream extends HTMLElement {
  connectedCallback() {
    this.start = this.querySelector("[data-start]");
    this.cancel = this.querySelector("[data-cancel]");
    this.status = this.querySelector("[role=status]");
    this.output = this.querySelector("[data-output]");
    this.start.onclick = () => this.open();
    this.cancel.onclick = () => this.stop("Cancelled");
    this.start.disabled = false;
    this.onLeave = () => this.stop("Interrupted");
    window.addEventListener("pagehide", this.onLeave);
  }

  disconnectedCallback() {
    window.removeEventListener("pagehide", this.onLeave);
    this.stop("Interrupted");
    this.start.onclick = null;
    this.cancel.onclick = null;
    this.start.disabled = true;
    this.cancel.disabled = true;
  }

  open() {
    if (!this.isConnected || this.source) return;
    const path = this.dataset.path;
    let url;
    try {
      url = new URL(path, location.href);
      if (!path.startsWith("/") || url.origin !== location.origin) throw new Error();
    } catch {
      this.status.textContent = "Invalid stream path";
      return;
    }
    const source = new EventSource(url);
    this.source = source;
    const answer = document.createTextNode("");
    this.output.replaceChildren(answer);
    this.status.textContent = "Loading";
    this.start.disabled = true;
    this.cancel.disabled = false;
    source.addEventListener("chunk", event => {
      if (source !== this.source) return;
      try {
        const chunk = JSON.parse(event.data);
        if (typeof chunk !== "string") throw new Error();
        answer.appendData(chunk);
        if (this.status.textContent !== "Streaming") this.status.textContent = "Streaming";
      } catch {
        this.stop("Invalid text response");
      }
    });
    source.addEventListener("completed", () => {
      if (source === this.source) this.stop("Complete");
    });
    source.addEventListener("failed", event => {
      if (source !== this.source) return;
      let detail = "the server stopped the stream";
      try {
        const failure = JSON.parse(event.data);
        if (typeof failure?.detail === "string") detail = failure.detail;
      } catch {}
      this.stop("Stream failed: " + detail);
    });
    source.onerror = () => {
      if (source === this.source) this.stop("Connection interrupted");
    };
  }

  stop(message) {
    if (!this.source) return;
    this.source.close();
    this.source = null;
    this.cancel.disabled = true;
    this.start.disabled = false;
    this.start.textContent = "Start again";
    this.status.textContent = message;
  }
}

customElements.define("corvid-text-stream", CorvidTextStream);
