// Exercise the *packaged* loader and Wasm against a recording adapter.
// The Playwright trial separately checks native browser discovery and keyboard events.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { text as readText } from "node:stream/consumers";

const [, , loader, kind, adapter] = process.argv;
const text = kind === "text";
const registered = new Map();
const alert = { textContent: "" };
const output = { value: "40" };
const tool = "record_page_read";
const controls = JSON.parse(await readText(process.stdin)).map(attrs => ({
  dataset: {
    corvidInput: attrs["data-corvid-input"],
    corvidAction: attrs["data-corvid-action"],
    corvidTool: attrs["data-corvid-tool"],
    corvidDescription: attrs["data-corvid-description"],
  },
  value: attrs.value,
  isConnected: true,
  disabled: Object.hasOwn(attrs, "disabled"),
  matches() { return this.disabled; },
  closest: () => panel,
  listeners: new Map(),
  addEventListener(event, callback) { this.listeners.set(event, callback); },
}));
const control = controls.find(element => element.dataset.corvidTool === tool);
assert.ok(control, "Expected the public binding in the served page");
const listeners = control.listeners;
const panel = {
  parentElement: { dataset: {} },
  querySelector: selector => {
    if (selector.includes("output")) return output;
    if (selector.includes("alert")) return alert;
    return text ? control : null;
  },
  querySelectorAll: () => controls,
};
const moduleUrl = pathToFileURL(loader);
globalThis.document = {
  querySelectorAll(selector) {
    if (selector.startsWith("script")) return [{ src: moduleUrl.href }];
    if (selector === "fieldset[data-corvid-state]") return [panel];
    return [];
  },
  modelContext: adapter === "absent" ? undefined : {
    registerTool: descriptor => registered.set(descriptor.name, descriptor),
  },
};
globalThis.fetch = async url => new Response(await readFile(url), {
  headers: { "Content-Type": "application/wasm" },
});
await import(moduleUrl);
for (let i = 0; control.disabled && i < 200; i++) {
  await new Promise(resolve => setTimeout(resolve, 10));
}
assert.equal(control.disabled, false, alert.textContent);
if (adapter === "absent") {
  assert.equal(registered.size, 0);
  listeners.get("click")();
  assert.equal(output.value, "41");
  process.exit(0);
}
assert.deepEqual([...registered.keys()], [tool], alert.textContent);
assert.ok(registered.get(tool).description);
const { inputSchema, outputSchema, execute } = registered.get(tool);
assert.equal(inputSchema.type, "object");
assert.equal(inputSchema.additionalProperties, false);
assert.equal(outputSchema.type, "string");
if (text) {
  assert.deepEqual(inputSchema.required, ["current"]);
  assert.deepEqual(Object.keys(inputSchema.properties), ["current"]);
  assert.equal(inputSchema.properties.current.type, "string");
  assert.equal(output.value, "first");
  control.value = "second";
  listeners.get("input")();
  assert.equal(output.value, "second");
  assert.equal(execute({ current: "third" }), "third");
  assert.equal(control.value, "third");
  for (const args of [{}, { current: 1 }, { current: "two\nlines" }, { current: "x", extra: 2 }]) {
    assert.throws(() => execute(args));
  }
  assert.equal(output.value, "third");
} else {
  assert.deepEqual(inputSchema.properties, {});
  listeners.get("click")();
  assert.equal(output.value, "41");
  assert.equal(execute({}), "42");
  assert.match(output.value, new RegExp(outputSchema.pattern));
  assert.throws(() => execute({ current: 0 }));
  assert.equal(output.value, "42");
}
control.disabled = true;
assert.throws(() => execute(text ? { current: "fourth" } : {}), /unavailable/);
control.disabled = false;
delete control.dataset.corvidTool;
assert.throws(() => execute(text ? { current: "fourth" } : {}), /unavailable/);
assert.equal(output.value, text ? "third" : "42");
if (!text) {
  const reset = controls.find(element => element.dataset.corvidAction === "reset_pages");
  assert.equal(registered.has("reset_pages"), false);
  reset.listeners.get("click")();
  assert.equal(output.value, "0");
}
