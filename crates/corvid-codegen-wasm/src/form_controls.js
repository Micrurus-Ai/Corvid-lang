// This module is loaded by interactive pages, including pages with no Wasm exports.
for (const select of document.querySelectorAll("select[data-corvid-submit-change]")) {
  select.addEventListener("change", () => select.form.requestSubmit());
}

for (const binding of document.querySelectorAll("[data-corvid-bind-field]")) {
  const field = binding.querySelector("input, select, textarea");
  const slot = binding.querySelector("[data-corvid-component]");
  const component = slot?.firstElementChild;
  if (!field || !component || !component.localName.includes("-")) continue;
  customElements.whenDefined(component.localName).then(() => {
    if (!("value" in component)) return;
    const sync = () => { if (component.value !== field.value) component.value = field.value; };
    sync();
    field.addEventListener("input", sync);
    field.addEventListener("change", sync);
    field.form?.addEventListener("reset", () => queueMicrotask(sync));
    component.addEventListener("change", () => {
      if (field.value === component.value) return;
      field.value = component.value;
      field.dispatchEvent(new Event("input", { bubbles: true }));
      field.dispatchEvent(new Event("change", { bubbles: true }));
    });
    slot.hidden = false;
  });
}

for (const form of document.querySelectorAll("form[data-corvid-receipt]")) {
  form.addEventListener("submit", event => {
    if (!event.agentInvoked) return;
    event.preventDefault();
    const send = async () => {
      const response = await fetch(form.action, {method: "POST", body: new URLSearchParams(new FormData(form))});
      const body = await response.text();
      const receipt = response.headers.get("content-type")?.includes("text/html")
        ? new DOMParser().parseFromString(body, "text/html").body.textContent.trim()
        : body;
      if (!response.ok) throw Error(receipt || response.statusText);
      form.querySelector("output[role=status]").textContent = receipt;
      return receipt;
    };
    event.respondWith(send());
  });
}

for (const form of document.querySelectorAll("form[data-corvid-selection]")) {
  const fields = Array.from(form.querySelectorAll('input[type="range"][name]'));
  const preview = form.querySelector(":scope > [data-corvid-preview]");
  const alert = form.querySelector(":scope > [role=alert]");
  const owner = form.parentElement;
  const properties = Object.create(null);
  for (const field of fields) {
    properties[field.name] = {type: "integer", minimum: Number(field.min), maximum: Number(field.max)};
  }
  if (!fields.length || Object.keys(properties).length !== fields.length) {
    alert.textContent = "A selection needs sliders with distinct names.";
    continue;
  }
  const read = () => Object.fromEntries(fields.map(field => [field.name, field.valueAsNumber]));
  let committed = read();
  let revision = 0;
  async function select(values) {
    if (!form.isConnected || fields.some(f => f.matches(":disabled"))) throw Error("Selection is unavailable.");
    if (!values || Array.isArray(values) || typeof values !== "object" ||
        Object.keys(values).length !== fields.length) throw Error("Expected all selection fields.");
    for (const field of fields) {
      const value = values[field.name];
      if (!Object.hasOwn(values, field.name) || !Number.isSafeInteger(value) ||
          value < Number(field.min) || value > Number(field.max)) throw Error("Invalid " + field.name + ".");
    }
    const action = new URL(form.action);
    if (action.origin !== location.origin) throw Error("Expected a same-origin selection route.");
    const next = Object.fromEntries(fields.map(field => [field.name, values[field.name]]));
    const current = ++revision;
    for (const field of fields) field.valueAsNumber = next[field.name];
    try {
      const response = await fetch(action, {method: "POST", body: new URLSearchParams(next), redirect: "error"});
      if (!response.ok || !response.headers.get("content-type")?.includes("text/html")) {
        throw Error("Could not update selection (" + response.status + ").");
      }
      const html = await response.text();
      if (current !== revision || !form.isConnected) throw Error("Selection was superseded.");
      const template = document.createElement("template");
      template.innerHTML = html;
      preview.replaceChildren(template.content);
      committed = next;
      alert.textContent = "";
      return preview.textContent.trim();
    } catch (error) {
      if (current === revision && form.isConnected) {
        for (const field of fields) field.valueAsNumber = committed[field.name];
        alert.textContent = error.message;
      }
      throw error;
    }
  }
  function update() { select(read()).catch(() => {}); }
  form.addEventListener("change", event => { if (fields.includes(event.target)) update(); });
  form.addEventListener("submit", event => { event.preventDefault(); update(); });
  form.querySelector(':scope > button[type="submit"]').hidden = true;
  if (owner.hasAttribute("data-corvid-selection-owner") && document.modelContext?.registerTool) {
    Promise.resolve().then(() => document.modelContext.registerTool({
      name: owner.dataset.corvidTool, description: owner.dataset.corvidDescription,
      inputSchema: {type: "object", properties, required: Object.keys(properties), additionalProperties: false},
      execute: select,
    })).catch(error => { alert.textContent = error.message; });
  }
}
