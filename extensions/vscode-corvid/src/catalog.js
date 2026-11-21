"use strict";

const vscode = require("vscode");

async function addPackageImport() {
  const editor = vscode.window.activeTextEditor;
  if (editor?.document.languageId !== "corvid") {
    return vscode.window.showInformationMessage("Open a Corvid source file to add a package import.");
  }
  const endpoint = vscode.workspace.getConfiguration("corvid").get("catalog.url");
  try {
    const response = await fetch(endpoint, {signal: AbortSignal.timeout(10000)});
    if (!response.ok) throw new Error("HTTP " + response.status);
    const entries = await response.json();
    const items = entries.map(pkg => {
      // Treat catalog data as untrusted text. Insert only a complete package import.
      if (!/^import "corvid:\/\/@[a-z0-9_-]+\/[a-z0-9_-]+\/v[0-9]+\.[0-9]+\.[0-9]+" as [A-Za-z_][A-Za-z_0-9]*$/.test(pkg.importLine)
          || /[\r\n]/.test(pkg.importLine)) throw new Error("Invalid package import");
      return {label: pkg.name, description: pkg.description, detail: pkg.keywords, importLine: pkg.importLine};
    });
    const choice = await vscode.window.showQuickPick(items, {
      placeHolder: "Find a package by name or task", matchOnDescription: true, matchOnDetail: true,
    });
    if (!choice || vscode.window.activeTextEditor !== editor) return;
    if (editor.document.getText().split(/\r?\n/).includes(choice.importLine)) return;
    await editor.edit(edit => edit.insert(new vscode.Position(0, 0),
      choice.importLine + (editor.document.eol === vscode.EndOfLine.CRLF ? "\r\n" : "\n")));
  } catch (error) {
    vscode.window.showErrorMessage("Could not load Corvid packages: " + error.message);
  }
}

module.exports = {addPackageImport};
