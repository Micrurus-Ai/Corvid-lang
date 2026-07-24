import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const jsonFiles = [
  "package.json",
  "language-configuration.json",
  "syntaxes/corvid.tmLanguage.json",
  "snippets/corvid.code-snippets"
];

for (const file of jsonFiles) {
  JSON.parse(readFileSync(resolve(root, file), "utf8"));
}

const manifest = JSON.parse(readFileSync(resolve(root, "package.json"), "utf8"));
if (!manifest.contributes?.languages?.some(
  (language) => language.id === "corvid" && language.extensions?.includes(".cor")
)) {
  throw new Error("Corvid extension must associate .cor files automatically");
}
if (!manifest.files?.includes("node_modules/**")) {
  throw new Error("VSIX package must include production language-client dependencies");
}
if (manifest.overrides?.["brace-expansion"] !== "2.1.2") {
  throw new Error("VSIX must retain the audited brace-expansion override");
}

const grammarText = readFileSync(
  resolve(root, "syntaxes/corvid.tmLanguage.json"),
  "utf8"
);
for (const scope of [
  "entity.name.function.corvid",
  "entity.name.function.call.corvid",
  "entity.name.type.corvid",
  "variable.other.property.corvid",
  "storage.modifier.annotation.corvid",
  "keyword.other.ai-native.corvid"
]) {
  if (!grammarText.includes(scope)) {
    throw new Error(`Corvid grammar is missing semantic scope: ${scope}`);
  }
}

execFileSync(process.execPath, ["--check", resolve(root, "src/extension.js")], {
  stdio: "inherit"
});

console.log("corvid-vscode extension verification passed");
