const projectionModels = new WeakMap();
let projectionHandlers;

function registerProjectionModel(region, properties) {
  const model = {
    properties,
    subscribers: new Set(),
    update(state) {
      this.state = state;
      for (const notify of this.subscribers) notify();
    },
  };
  projectionModels.set(region, model);
  return model;
}

// Compute and validate the full key sequence before changing the live list.
function renderProjectedRows(element, keys) {
  const list = element.firstElementChild;
  const template = list.nextElementSibling;
  const existing = new Map(Array.from(list.children, row => [row.dataset.corvidKey, row]));
  const ordered = keys.map(key => {
    const retained = existing.get(key);
    if (retained) {
      existing.delete(key);
      return retained;
    }
    const row = template.content.firstElementChild.cloneNode(true);
    row.dataset.corvidKey = key;
    for (const label of row.querySelectorAll("[data-projected-key]")) label.textContent = key;
    return row;
  });
  for (const row of existing.values()) row.remove();

  placeOrderedRows(list, ordered);
}

if (!customElements.get("corvid-record-projection")) {
  customElements.define("corvid-record-projection", class extends HTMLElement {
    #unsubscribe;
    #mount;

    async connectedCallback() {
      const mount = {};
      this.#mount = mount;
      try {
        projectionHandlers ??= instantiate().catch(error => {
          projectionHandlers = undefined;
          throw error;
        });
        const handlers = await projectionHandlers;
        if (this.#mount !== mount) return;
        const model = projectionModels.get(this.closest("[data-local-selection]"));
        if (!model) throw Error("A projection needs a mounted browser record.");
        const fields = this.dataset.fields.split(" ").filter(Boolean);
        const kinds = fields.map(field => {
          const type = model.properties[field]?.type;
          if (!type) throw Error("Unknown projection field " + field);
          return {integer: "i64", boolean: "i32", string: "string"}[type];
        });
        function handler(name, inputs, outputs) {
          const signature = projectionSignatures.get(name);
          if (!signature || !outputs.includes(signature[1]) || signature[0].length !== inputs.length ||
              inputs.some((kind, index) => kind !== signature[0][index])) {
            throw Error("Incompatible projection handler " + name);
          }
          return handlers[name];
        }
        const list = this.hasAttribute("data-key-at");
        const condition = this.hasAttribute("data-condition");
        const project = handler(this.dataset.handler, kinds,
          list ? ["i64"] : condition ? ["i32"] : ["string", "i64", "i32", "f64"]);
        const keyAt = list && handler(this.dataset.keyAt, [...kinds, "i64"], ["string", "i64"]);
        const render = () => {
          const args = fields.map((field, index) =>
            kinds[index] === "i64" ? BigInt(model.state[field]) : model.state[field]);
          const result = project(...args);
          if (list) {
            const keys = [];
            const seen = new Set();
            if (result < 0n || result > BigInt(Number.MAX_SAFE_INTEGER)) throw Error("Invalid row count.");
            for (let index = 0n; index < result; index++) {
              const key = String(keyAt(...args, index));
              if (!key || seen.has(key)) throw Error("Expected a unique row key: " + key);
              keys.push(key);
              seen.add(key);
            }
            renderProjectedRows(this, keys);
          } else if (condition) {
            this.hidden = !result;
          } else if (this.firstElementChild.value !== String(result)) {
            this.firstElementChild.value = String(result);
          }
        };
        let pending = false;
        const notify = () => {
          if (pending) return;
          pending = true;
          queueMicrotask(() => {
            pending = false;
            if (this.#mount !== mount) return;
            try { render(); }
            catch (error) { this.reportError(error); }
          });
        };
        model.subscribers.add(notify);
        this.#unsubscribe = () => model.subscribers.delete(notify);
        render();
      } catch (error) {
        if (this.#mount === mount) this.reportError(error);
      }
    }

    reportError(error) {
      const alert = this.closest("[data-local-selection]")?.querySelector(":scope > [role=alert]");
      if (alert) alert.textContent = error.message;
    }

    disconnectedCallback() {
      this.#mount = undefined;
      this.#unsubscribe?.();
      this.#unsubscribe = undefined;
    }
  });
}
