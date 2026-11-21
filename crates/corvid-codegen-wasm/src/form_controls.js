// This module is loaded by interactive pages, including pages with no Wasm exports.
for (const select of document.querySelectorAll("select[data-corvid-submit-change]")) {
  select.addEventListener("change", () => select.form.requestSubmit());
}
