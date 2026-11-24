function quantityBinding(field) {
  const input = field.querySelector(":scope > label > input[data-corvid-quantity]");
  const output = field.querySelector(":scope > label > output");
  const alert = field.querySelector(":scope > [role=alert]");
  const name = input?.dataset.corvidQuantity;
  if (!input || !output || !alert) {
    const location = handlerLocations.get(name) ?? "std/ui.quantity_field";
    const message = location + ": expected a quantity input, result output and alert.";
    if (alert) alert.textContent = message;
    else console.error(message, field);
    return;
  }
  if (!intStateHandlers.has(name)) {
    alert.textContent = "Expected a Corvid handler " + name + "(Int) -> Int.";
    return;
  }
  return {input, output, alert, name};
}

function bindQuantity({input, output, alert, name}, handlers) {
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
