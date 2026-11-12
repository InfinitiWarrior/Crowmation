// Starts `crowmation lsp` for .cwm files.
const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const command = vscode.workspace.getConfiguration("crowmation").get("path") || "crowmation";
  client = new LanguageClient(
    "crowmation",
    "Crowmation",
    { command, args: ["lsp"] },
    { documentSelector: [{ scheme: "file", language: "cwm" }] }
  );
  client.start();
  context.subscriptions.push(client);
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
