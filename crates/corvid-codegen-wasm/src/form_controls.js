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
