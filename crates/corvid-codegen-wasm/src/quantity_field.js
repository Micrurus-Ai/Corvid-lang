function bindQuantity(field, handlers, intStateHandlers) {
  const input = field.querySelector("input");
  const output = field.querySelector("output");
  const alert = field.querySelector("[role=alert]");
  const name = input.dataset.corvidQuantity;
  if (!intStateHandlers.has(name)) {
    alert.textContent = "Expected a Corvid handler " + name + "(Int) -> Int.";
    return;
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
