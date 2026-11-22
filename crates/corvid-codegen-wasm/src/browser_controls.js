// A module script marked by std/ui opts into binding its page's controls.
if (typeof document !== "undefined" && Array.from(
  document.querySelectorAll("script[type=module][data-corvid-ui]"),
).some(script => script.src === import.meta.url)) {
  bindStateControls();
}

async function bindStateControls() {
  for (const select of document.querySelectorAll("select[data-corvid-submit-change]")) {
    select.addEventListener("change", () => select.form.requestSubmit());
  }
  for (const region of document.querySelectorAll("[data-corvid-list]")) bindKeyedList(region);
  const panels = document.querySelectorAll("fieldset[data-corvid-state]");
  const readers = new Map();
  for (const output of document.querySelectorAll("output[data-corvid-read]")) {
    const name = output.dataset.corvidRead;
    if (!readers.has(name)) readers.set(name, []);
    readers.get(name).push(output);
  }
  let handlers;
  try {
    handlers = await instantiate();
  } catch (error) {
    for (const alert of document.querySelectorAll("fieldset[data-corvid-state] > [role=alert], [data-corvid-quantity-field] > [role=alert]")) {
      alert.textContent = "Could not load Corvid event handlers: " + error.message;
    }
    return;
  }
  for (const field of document.querySelectorAll("[data-corvid-quantity-field]")) {
    const input = field.querySelector("input");
    const output = field.querySelector("output");
    const alert = field.querySelector("[role=alert]");
    const name = input.dataset.corvidQuantity;
    if (!intStateHandlers.has(name)) {
      alert.textContent = "Expected a Corvid handler " + name + "(Int) -> Int.";
      continue;
    }
    const update = () => {
      try {
        output.value = input.validity.valid ? handlers[name](BigInt(input.value)).toString() : "—";
        alert.textContent = "";
      } catch (error) {
        output.value = "—";
        alert.textContent = "Corvid handler " + name + " failed: " + error.message;
      }
    };
    input.addEventListener("input", update);
    update();
    output.parentElement.hidden = false;
  }
  for (const panel of panels) {
    const output = panel.querySelector(":scope > output");
    const alert = panel.querySelector(":scope > [role=alert]");
    const input = panel.querySelector(":scope > label > input[data-corvid-input]");
    if (input) {
      const name = input.dataset.corvidInput;
      if (!textStateHandlers.has(name)) {
        alert.textContent = "Expected a Corvid handler " + name + "(String) -> String.";
        continue;
      }
      const invoke = value => {
        const result = handlers[name](value);
        input.value = value;
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

function bindKeyedList(region) {
  const list = region.querySelector(":scope > ol");
  const status = region.querySelector(":scope > [role=status]");
  const rows = new Map();
  for (const row of list.children) {
    const key = row.dataset.corvidKey;
    if (!key || rows.has(key)) {
      status.textContent = "Expected a unique key in " + list.getAttribute("aria-label") + ": " + (key ?? "(missing)");
      return;
    }
    rows.set(key, row);
  }
  function move(row, direction) {
    const up = direction === "up";
    const neighbor = up ? row.previousElementSibling : row.nextElementSibling;
    if (neighbor) {
      // Detach only the unfocused sibling, including when a pointer triggers the move.
      if (neighbor.contains(document.activeElement)) {
        list.insertBefore(row, up ? neighbor : neighbor.nextElementSibling);
      } else {
        list.insertBefore(neighbor, up ? row.nextElementSibling : row);
      }
    }
    const position = Array.from(list.children).indexOf(row) + 1;
    status.textContent = row.dataset.corvidKey + ", position " + position + " of " + rows.size;
  }
  for (const [key, row] of rows) {
    for (const button of row.querySelectorAll(":scope > button[data-corvid-move]")) {
      button.addEventListener("click", () => move(rows.get(key), button.dataset.corvidMove));
      button.hidden = false;
    }
    row.addEventListener("keydown", event => {
      if (event.target.closest("[data-corvid-list]") !== region || event.isComposing ||
          !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
      if (event.key === "ArrowUp" || event.key === "ArrowDown") {
        event.preventDefault();
        move(rows.get(key), event.key === "ArrowUp" ? "up" : "down");
      }
    });
  }
}
