// The server emits JSON strings followed by a named done or error event.
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
    source.onmessage = event => {
      if (source !== this.source) return;
      try {
        const chunk = JSON.parse(event.data);
        if (typeof chunk !== "string") throw new Error();
        answer.appendData(chunk);
        if (this.status.textContent !== "Streaming") this.status.textContent = "Streaming";
      } catch {
        this.stop("Invalid text response");
      }
    };
    source.addEventListener("done", () => {
      if (source === this.source) this.stop("Complete");
    });
    source.onerror = event => {
      if (source === this.source) {
        this.stop(event instanceof MessageEvent ? "Stream failed: " + event.data : "Connection interrupted");
      }
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
