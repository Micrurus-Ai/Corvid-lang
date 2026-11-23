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

// Each mounted record owns its value and bindings. Only initialization reads the schema markup.
for (const region of document.querySelectorAll("[data-local-selection]")) {
  const alert = region.querySelector(":scope > [role=alert]");
  try {
    bindLocalSelection(region);
  } catch (error) {
    alert.textContent = error.message;
  }
}

function bindLocalSelection(region) {
  const members = Array.from(region.querySelectorAll(":scope > [data-field]"));
  const initial = Object.create(null);
  const properties = Object.create(null);
  for (const {dataset} of members) {
    const {field, min, max, value} = dataset;
    const minimum = Number(min), maximum = Number(max);
    if (Object.hasOwn(initial, field) || !Number.isSafeInteger(minimum) ||
        !Number.isSafeInteger(maximum) || minimum > maximum) throw Error("Invalid record field: " + field);
    properties[field] = {type: "integer", minimum, maximum};
    initial[field] = Number(value);
  }
  for (const node of region.querySelectorAll(":scope > [data-record-field]")) {
    const {name, schema, value} = JSON.parse(node.dataset.recordField);
    if (Object.hasOwn(initial, name)) throw Error("Duplicate record field: " + name);
    properties[name] = schema;
    initial[name] = value;
  }
  const names = Object.keys(initial);
  const owned = selector => Array.from(region.querySelectorAll(selector))
    .filter(node => node.closest("[data-local-selection]") === region);
  for (const slot of owned("[data-local-field]")) {
    const name = slot.dataset.localField;
    const spec = properties[name];
    if (!spec) throw Error("Unknown record field: " + name);
    let control;
    if (spec.enum) {
      control = document.createElement("select");
      for (const value of spec.enum) control.add(new Option(value, value));
    } else if (spec.type === "string") {
      control = document.createElement("textarea");
      control.rows = 1;
    } else {
      control = document.createElement("input");
      control.type = spec.type === "boolean" ? "checkbox" : "range";
    }
    control.name = name;
    control.disabled = true;
    slot.replaceWith(control);
  }
  const groups = owned("[data-choice-group]");
  for (const group of groups) {
    const field = group.dataset.choiceGroup;
    const options = properties[field]?.enum;
    if (!options) throw Error("A segmented choice needs a choice field: " + field);
    // A distinct native radio group for every mounted instance.
    const radioName = "corvid-choice-" + crypto.getRandomValues(new Uint32Array(4)).join("-");
    const row = group.querySelector(":scope > [data-options]");
    for (const option of options) {
      const label = document.createElement("label");
      label.style.padding = "0.35rem 0.6rem";
      const radio = document.createElement("input");
      radio.type = "radio";
      radio.name = radioName;
      radio.value = option;
      label.append(radio, document.createTextNode(option));
      row.append(label);
    }
  }
  const controls = [...owned('input[name]:not([type=radio]),select[name],textarea[name]'), ...groups];
  const boundName = control => control.dataset.choiceGroup ?? control.name;
  if (!names.length || controls.length !== names.length ||
      names.some(name => !controls.some(input => boundName(input) === name))) {
    throw Error("Each record field needs one matching control.");
  }
  const preview = region.querySelector(":scope > [data-local-preview]");
  const placeholder = /\{([^{}]+)\}/g;
  function fieldName(name) {
    if (!Object.hasOwn(properties, name)) throw Error("Unknown record field: " + name);
    return name;
  }
  const texts = owned("[data-record-text]").map(node => {
    const pattern = node.dataset.recordText;
    for (const match of pattern.matchAll(placeholder)) fieldName(match[1]);
    return {node, pattern};
  });
  const samples = owned("[data-record-swatch]").map(node => {
    const channels = [node.dataset.red, node.dataset.green, node.dataset.blue].map(fieldName);
    if (channels.some(name => properties[name].type !== "integer")) {
      throw Error("A color swatch needs three integer fields.");
    }
    return {node, channels};
  });
  let state;
  function select(values) {
    if (!values || Array.isArray(values) || typeof values !== "object" ||
        Object.keys(values).length !== names.length) throw Error("Expected all record fields.");
    const next = Object.create(null);
    for (const name of names) {
      const value = values[name];
      const spec = properties[name];
      let valid = false;
      if (spec.type === "integer") {
        valid = Number.isSafeInteger(value) && value >= spec.minimum && value <= spec.maximum;
      } else if (spec.type === "boolean") {
        valid = typeof value === "boolean";
      } else if (spec.type === "string" && typeof value === "string") {
        const length = Array.from(value).length;
        valid = (!spec.enum || spec.enum.includes(value)) &&
          length >= (spec.minLength ?? 0) && length <= (spec.maxLength ?? Infinity);
      }
      if (!Object.hasOwn(values, name) || !valid) throw Error("Invalid " + name + ".");
      next[name] = value;
    }
    state = Object.freeze(next);
    for (const control of controls) {
      const value = state[boundName(control)];
      if (control.hasAttribute("data-choice-group")) {
        for (const radio of control.querySelectorAll('input[type=radio]')) radio.checked = radio.value === value;
      } else if (control.type === "checkbox") control.checked = value;
      else if (control.value !== String(value)) control.value = value;
    }
    region.querySelector(":scope > [role=alert]").textContent = "";
    for (const {node, pattern} of texts) {
      node.textContent = pattern.replace(placeholder, (_, name) => state[name]);
    }
    for (const {node, channels} of samples) {
      node.style.backgroundColor = "rgb(" + channels.map(name => state[name]).join(",") + ")";
    }
    return preview.textContent.trim();
  }
  for (const control of controls) {
    if (control.hasAttribute("data-choice-group")) continue;
    const spec = properties[control.name];
    const expected = spec.enum ? "select-one" :
      {boolean: "checkbox", string: "textarea", integer: "range"}[spec.type];
    if (control.type !== expected) throw Error("Incorrect control for " + control.name);
    if (control.type === "range") {
      control.min = spec.minimum;
      control.max = spec.maximum;
    }
  }
  select(initial);
  for (const control of controls) {
    control.disabled = false;
    control.addEventListener("input", event => {
      const value = control.hasAttribute("data-choice-group") ? event.target.value :
        control.type === "checkbox" ? control.checked :
        control.type === "range" ? control.valueAsNumber : control.value;
      try { select({...state, [boundName(control)]: value}); }
      catch (error) { region.querySelector(":scope > [role=alert]").textContent = error.message; }
    });
  }
  const owner = region.parentElement;
  if (owner.hasAttribute("data-corvid-selection-owner") && document.modelContext?.registerTool) {
    Promise.resolve().then(() => document.modelContext.registerTool({
      name: owner.dataset.corvidTool, description: owner.dataset.corvidDescription,
      inputSchema: {type: "object", properties, required: names, additionalProperties: false},
      execute(values) {
        if (!region.isConnected || controls.some(input => input.matches(":disabled"))) {
          throw Error("Selection is unavailable.");
        }
        return select(values);
      },
    })).catch(error => { region.querySelector(":scope > [role=alert]").textContent = error.message; });
  }
}
