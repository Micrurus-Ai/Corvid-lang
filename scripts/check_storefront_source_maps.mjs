import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { SourceMap } from "node:module";
import { join } from "node:path";

const project = process.argv[2];
const source = readFileSync(join(project, "src/events.cor"), "utf8").split(/\r?\n/);
for (const [asset, instruction, original] of [
  ["events.js", "const result = exports.notebook_cents(", "public agent notebook_cents("],
  ["events.js", "p0 * (450n)", "return quantity * 450"],
  ["events.quantity.js", "p0 * (450n)", "return quantity * 450"],
]) {
  const code = readFileSync(join(project, "target/wasm", asset), "utf8");
  const encoded = code.split("//# sourceMappingURL=data:application/json;base64,")[1];
  const map = new SourceMap(JSON.parse(Buffer.from(encoded, "base64").toString("utf8")));
  const generatedLine = code.split("\n").findIndex(line => line.includes(instruction));
  const originalLine = source.findIndex(line => line.includes(original));
  assert.ok(generatedLine >= 0 && originalLine >= 0, instruction);
  const entry = map.findEntry(generatedLine, 4);
  assert.equal(entry.generatedLine, generatedLine);
  assert.equal(entry.originalSource, "src/events.cor");
  assert.equal(entry.originalLine, originalLine);
}
console.log("Packaged JavaScript wrapper and quantity calculations map to their Corvid source lines");
