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
// brace-expansion is overridden to a release past its denial-of-service
// advisories (GHSA-q2hr-2g5m-vwhr, fixed in 2.1.7, is the latest). The
// override must stay at or above that release, and the lockfile the VSIX
// is built from must resolve every copy to it.
const patchedBraceExpansion = [2, 1, 7];
const override = manifest.overrides?.["brace-expansion"];
const parts = /^(\d+)\.(\d+)\.(\d+)$/.exec(override ?? "")?.slice(1).map(Number);
const atLeast = (version, floor) => {
  for (let i = 0; i < floor.length; i++) {
    if (version[i] !== floor[i]) return version[i] > floor[i];
  }
  return true;
};
if (!parts || parts[0] !== 2 || !atLeast(parts, patchedBraceExpansion)) {
  throw new Error(`VSIX must override brace-expansion to an exact 2.x release at or above ${patchedBraceExpansion.join(".")}, found ${override}`);
}
const lock = JSON.parse(readFileSync(resolve(root, "package-lock.json"), "utf8"));
for (const [path, entry] of Object.entries(lock.packages ?? {})) {
  if (path.endsWith("node_modules/brace-expansion") && entry.version !== override) {
    throw new Error(`package-lock.json resolves ${path} to ${entry.version}, not the override ${override}`);
  }
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
