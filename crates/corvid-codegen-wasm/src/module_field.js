function moduleFieldElement(template) {
  const { corvidModuleField: modulePath, reader, handler } = template.content.firstElementChild.dataset;
  return class extends HTMLElement {
    #value = "";
    #revision = 0;
    #handlers;

    get value() { return this.#value; }

    set value(text) {
      this.#revision++;
      this.#value = String(text);
      if (this.shadowRoot) {
        this.shadowRoot.firstElementChild.querySelector(":scope > output").value = this.#value;
        this.shadowRoot.firstElementChild.querySelector(":scope > [role=alert]").textContent = "";
      }
    }

    connectedCallback() {
      if (this.shadowRoot) return;
      const root = this.attachShadow({ mode: "open" });
      root.append(template.content.cloneNode(true));
      // A host may have set the property before customElements.define().
      if (Object.hasOwn(this, "value")) {
        const initial = this.value;
        delete this.value;
        this.value = initial;
      }
      root.firstElementChild.querySelector(":scope > output").value = this.value;
      root.querySelector("[data-corvid-control]").addEventListener("change", event => {
        event.stopPropagation();
        this.#read(event.target);
      });
    }

    disconnectedCallback() { this.#revision++; }

    async #read(control) {
      const revision = ++this.#revision;
      const alert = this.shadowRoot.firstElementChild.querySelector(":scope > [role=alert]");
      alert.textContent = "";
      try {
        const value = control instanceof HTMLInputElement && control.type === "file"
          ? control.files.item(0) : control.value;
        if (value !== null && typeof value !== "string" && !(value instanceof File)) {
          throw TypeError("Expected a control with a string value or a file input");
        }
        if (!textStateHandlers.has(handler)) throw Error("Expected a (String) -> String handler");
        const module = await import(modulePath);
        if (typeof module[reader] !== "function") throw Error("Missing module function " + reader);
        this.#handlers ??= instantiate().catch(error => {
          this.#handlers = undefined;
          throw error;
        });
        const handlers = await this.#handlers;
        if (revision !== this.#revision) return;
        const convert = text => {
          if (typeof text !== "string") throw TypeError(handler + " expects a string");
          return handlers[handler](text);
        };
        const result = await module[reader](value, convert);
        if (typeof result !== "string") throw TypeError(reader + " must return a string");
        if (revision !== this.#revision) return;
        this.value = result;
        this.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
      } catch (error) {
        if (revision !== this.#revision) return;
        const location = handlerLocations.get(handler);
        alert.textContent = `${reader} → ${handler}${location ? " at " + location : ""}: ${error?.message ?? error}`;
      }
    }
  };
}
