// A module script marked by std/ui opts into binding its page's controls.
if (typeof document !== "undefined" && Array.from(
  document.querySelectorAll("script[type=module][data-corvid-ui]"),
).some(script => script.src === import.meta.url)) {
  bindStateControls();
}

// Each host decides the tag name and can mount the class more than once.
export function componentElement(html) {
  if (html.includes("data-corvid-module-field")) {
    const template = document.createElement("template");
    template.innerHTML = html;
    if (template.content.childElementCount === 1 &&
        template.content.firstElementChild.matches("[data-corvid-module-field]")) {
      return moduleFieldElement(template);
    }
  }
  return class extends HTMLElement {
    connectedCallback() {
      if (this.shadowRoot) return;
      const root = this.attachShadow({ mode: "open" });
      const content = document.createElement("div");
      content.innerHTML = html;
      root.append(content);
      bindStateControls(content);
    }
  };
}

async function bindStateControls(root = document) {
  for (const region of root.querySelectorAll("[data-corvid-list]")) bindKeyedList(region);
  const panels = root.querySelectorAll("fieldset[data-corvid-state]");
  const dragRegions = root.querySelectorAll("[data-corvid-drag]");
  const quantities = root.querySelectorAll("[data-corvid-quantity-field]");
  if (!panels.length && !dragRegions.length && !quantities.length) return;
  const readers = new Map();
  for (const output of root.querySelectorAll("output[data-corvid-read]")) {
    const name = output.dataset.corvidRead;
    if (!readers.has(name)) readers.set(name, []);
    readers.get(name).push(output);
  }
  let handlers;
  try {
    handlers = await instantiate();
  } catch (error) {
    for (const alert of root.querySelectorAll("[data-corvid-drag] > [role=alert], fieldset[data-corvid-state] > [role=alert], [data-corvid-quantity-field] > [role=alert]")) {
      alert.textContent = "Could not load Corvid event handlers: " + error.message;
    }
    return;
  }
  if (typeof bindDrag === "function") {
    for (const region of dragRegions) bindDrag(region, handlers);
  }
  for (const field of quantities) {
    const binding = quantityBinding(field);
    if (binding) bindQuantity(binding, handlers);
  }
  for (const panel of panels) {
    const output = panel.querySelector(":scope > output");
    const alert = panel.querySelector(":scope > [role=alert]");
    const input = panel.querySelector(":scope > label > input[data-corvid-input]");
    if (!output || !alert || (!input && !/^-?\d+$/.test(output.value))) {
      const handler = input?.dataset.corvidInput ?? panel.querySelector("button[data-corvid-action]")?.dataset.corvidAction;
      const location = handlerLocations.get(handler) ?? "std/ui.state";
      const message = location + ": expected a state output and an alert for this control.";
      if (alert) alert.textContent = message;
      else console.error(message, panel);
      continue;
    }
    if (input) {
      const name = input.dataset.corvidInput;
      if (!textStateHandlers.has(name)) {
        alert.textContent = "Expected a Corvid handler " + name + "(String) -> String.";
        continue;
      }
      const invoke = value => {
        const result = handlers[name](value);
        if (input.value !== value) input.value = value;
        output.value = result;
        alert.textContent = "";
        return result;
      };
      const update = () => {
        try {
          invoke(input.value);
        } catch (error) {
          alert.textContent = "Corvid handler " + name + " failed: " + error.message;
        }
      };
      input.addEventListener("input", update);
      update();
      input.disabled = false;
      if (!input.dataset.corvidTool) continue;
      const schemas = textStateHandlers.get(name);
      registerControlTool(input, panel, schemas, alert, args => {
        const [field] = schemas.inputSchema.required;
        if (Object.keys(args).length !== 1 || !Object.hasOwn(args, field) ||
            typeof args[field] !== "string" || /[\r\n]/.test(args[field])) {
          throw new Error("Expected one single-line text field: " + field);
        }
        return invoke(args[field]);
      });
      continue;
    }
    let value = BigInt(output.value);
    const name = panel.parentElement.dataset.corvidOwner;
    const mirrors = readers.get(name) ?? [];
    for (const mirror of mirrors) mirror.value = output.value;
    for (const button of panel.querySelectorAll("button[data-corvid-action]")) {
      if (button.closest("fieldset[data-corvid-state]") !== panel) continue;
      const name = button.dataset.corvidAction;
      if (!intStateHandlers.has(name)) {
        alert.textContent = "Expected a Corvid handler " + name + "(Int) -> Int.";
        continue;
      }
      const invoke = () => {
        if (button.matches(":disabled") || !button.isConnected ||
            button.closest("fieldset[data-corvid-state]") !== panel) {
          throw new Error("Action is unavailable.");
        }
        value = handlers[name](value);
        output.value = value.toString();
        for (const mirror of mirrors) mirror.value = output.value;
        alert.textContent = "";
        return output.value;
      };
      button.addEventListener("click", () => {
        try {
          invoke();
        } catch (error) {
          alert.textContent = "Corvid handler " + name + " failed: " + error.message;
        }
      });
      button.disabled = false;
      if (!button.dataset.corvidTool) continue;
      registerControlTool(button, panel, intStateHandlers.get(name), alert, args => {
        if (Object.keys(args).length) throw new Error("Expected no action arguments.");
        return invoke();
      });
    }
  }
}

async function registerControlTool(control, panel, schemas, alert, invoke) {
  const tool = control.dataset.corvidTool;
  if (!tool || !document.modelContext?.registerTool) return;
  try {
    await document.modelContext.registerTool({
      name: tool,
      description: control.dataset.corvidDescription,
      ...schemas,
      execute(args) {
        if (!control.isConnected || control.matches(":disabled") ||
            control.closest("fieldset[data-corvid-state]") !== panel || control.dataset.corvidTool !== tool) {
          throw new Error("Action is unavailable.");
        }
        if (!args || typeof args !== "object" || Array.isArray(args)) throw new Error("Expected an object.");
        return invoke(args);
      },
    });
  } catch (error) {
    alert.textContent = "Could not register assistant action " + tool + ": " + error.message;
  }
}
