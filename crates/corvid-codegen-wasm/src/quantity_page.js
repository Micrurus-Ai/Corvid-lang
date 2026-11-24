async function bindQuantityPage() {
  const fields = document.querySelectorAll("[data-corvid-quantity-field]");
  for (const list of document.querySelectorAll("[data-corvid-list]")) bindKeyedList(list);
  const pending = [];
  for (const field of fields) {
    const binding = quantityBinding(field);
    if (!binding) continue;
    if (Object.hasOwn(integerHandlers, binding.name)) bindQuantity(binding, integerHandlers);
    else pending.push(binding);
  }
  if (!pending.length) return;
  try {
    const handlers = await (await import(runtimeUrl)).instantiate();
    for (const binding of pending) bindQuantity(binding, handlers);
  } catch (error) {
    for (const binding of pending) binding.alert.textContent =
      "Could not load Corvid event handlers: " + error.message;
  }
}

bindQuantityPage();
