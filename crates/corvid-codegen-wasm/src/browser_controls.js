// A module script marked by std/ui opts into binding its page's controls.
if (typeof document !== "undefined" && Array.from(
  document.querySelectorAll("script[type=module][data-corvid-ui]"),
).some(script => script.src === import.meta.url)) {
  bindStateControls();
}

async function bindStateControls() {
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
    for (const panel of panels) {
      panel.querySelector(":scope > [role=alert]").textContent =
        "Could not load Corvid event handlers: " + error.message;
    }
    return;
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
      const update = () => {
        try {
          output.value = handlers[name](input.value);
          alert.textContent = "";
        } catch (error) {
          alert.textContent = "Corvid handler " + name + " failed: " + error.message;
        }
      };
      input.addEventListener("input", update);
      update();
      input.disabled = false;
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
      button.addEventListener("click", () => {
        try {
          value = handlers[name](value);
          output.value = value.toString();
          for (const mirror of mirrors) mirror.value = output.value;
        } catch (error) {
          alert.textContent = "Corvid handler " + name + " failed: " + error.message;
        }
      });
      button.disabled = false;
    }
  }
}
