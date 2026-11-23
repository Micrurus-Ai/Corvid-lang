async function bindQuantityPage() {
  const fields = document.querySelectorAll("[data-corvid-quantity-field]");
  for (const list of document.querySelectorAll("[data-corvid-list]")) bindKeyedList(list);
  let handlers = integerHandlers;
  if (Array.from(fields).some(field => !Object.hasOwn(handlers, field.querySelector("input").dataset.corvidQuantity))) {
    try {
      handlers = await (await import("./events.js")).instantiate();
    } catch (error) {
      for (const field of fields) field.querySelector("[role=alert]").textContent =
        "Could not load Corvid event handlers: " + error.message;
      return;
    }
  }
  for (const field of fields) bindQuantity(field, handlers, intStateHandlers);
}

bindQuantityPage();
