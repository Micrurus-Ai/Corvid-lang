const assert = require("node:assert/strict");
const vscode = require("vscode");

exports.run = async () => {
  const file = vscode.Uri.joinPath(vscode.workspace.workspaceFolders[0].uri, "src", "main.cor");
  const document = await vscode.workspace.openTextDocument(file);
  await vscode.window.showTextDocument(document);
  const extension = vscode.extensions.getExtension("corvid-lang.corvid");
  await extension.activate();
  const search = vscode.commands.executeCommand("corvid.addPackageImport");
  // Drive the Quick Pick in the extension host just as Enter does in the UI.
  const accept = setInterval(() => {
    vscode.commands.executeCommand("workbench.action.acceptSelectedQuickOpenItem");
  }, 300);
  try {
    await search;
  } finally {
    clearInterval(accept);
  }
  assert.equal(document.lineAt(0).text, 'import "corvid://@corvid/ui/v0.0.1" as ui');
  assert(await document.save());
};
