import { connectionForPage } from "./live-connection.js";
import { fillPath } from "./path.js";

const connection = connectionForPage(location.origin);

// The CLI appends the declaration map from the checked Application Contract.
class CorvidLiveList extends HTMLElement {
  #release;
  #mount;

  connectedCallback() {
    const mount = this.#mount = {};
    const list = this.querySelector(":scope > ol");
    const template = this.querySelector(":scope > template");
    const status = this.querySelector(":scope > [role=status]");
    let nodes = new Map();
    const fail = error => {
      if (this.#mount !== mount) return;
      this.#release?.();
      list.replaceChildren();
      nodes.clear();
      status.textContent = error.message;
    };
    try {
      const route = liveDeclarations.get(JSON.stringify([this.dataset.server, this.dataset.live]));
      if (!route) throw Error("Unknown live declaration.");
      const args = JSON.parse(this.dataset.arguments);
      if (!Array.isArray(args) || args.length !== route.path_params.length) throw Error("Invalid live path arguments.");
      const values = Object.fromEntries(route.path_params.map(({name}, i) => [name, args[i]]));
      this.#release = connection.subscribe(fillPath(route.path, values), route.key_fields, {
        rows: rows => {
          if (this.#mount !== mount) return;
          try {
            const next = new Map();
            for (const row of rows) {
              const node = nodes.get(row) ?? this.renderRow(template, row);
              next.set(row, node);
            }
            for (const [row, node] of nodes) if (!next.has(row)) node.remove();
            placeOrderedRows(list, [...next.values()]);
            nodes = next;
            status.textContent = "";
          } catch (error) { fail(error); }
        },
        error: fail,
      });
    } catch (error) { fail(error); }
  }

  renderRow(template, row) {
    const node = template.content.firstElementChild.cloneNode(true);
    for (const output of node.querySelectorAll("[data-record-text]")) {
      output.textContent = output.dataset.recordText.replace(/\{([^{}]+)\}/g, (_, path) => {
        let value = row;
        for (const field of path.split(".")) {
          if (!value || !Object.hasOwn(value, field)) throw Error("Missing live field " + path);
          value = value[field];
        }
        return typeof value === "object" ? JSON.stringify(value) : String(value);
      });
    }
    return node;
  }

  disconnectedCallback() {
    this.#mount = undefined;
    this.#release?.();
    this.#release = undefined;
    this.querySelector(":scope > ol")?.replaceChildren();
  }
}
