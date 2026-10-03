// Runs inside VS Code's extension host.

import assert from "node:assert/strict";
import * as vscode from "vscode";

import type { TestApi } from "../../src/extension";

async function until<T>(get: () => T | undefined, timeoutMs: number, what: string): Promise<T> {
  const end = Date.now() + timeoutMs;
  for (;;) {
    const v = get();
    if (v !== undefined) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 100));
  }
}

export async function run(): Promise<void> {
  const file = process.env.CRT_TEST_FILE;
  assert.ok(file, "CRT_TEST_FILE not set");
  const doc = await vscode.workspace.openTextDocument(file);
  await vscode.window.showTextDocument(doc);

  const ext = vscode.extensions.getExtension<TestApi>("esh2n.code-reading");
  assert.ok(ext, "extension not found");
  const api = await ext.activate();
  const uri = doc.uri.toString();

  // Opening shows structural facts; viewing reads both functions. The
  // notes of Inc are shown while they arrive, one at a time.
  const partial = await until(
    () => (api.sawPartial(uri) ? true : undefined),
    30000,
    "notes shown while they arrive",
  );
  assert.ok(partial);
  const params = await api.waitForReadings(uri, 30000);
  assert.equal(params.readings[0]?.scenarios, null, "scenarios wait to be asked for");
  assert.deepEqual(
    params.analysis.functions.map((f) => [f.name, f.enclosing]),
    [
      ["Inc", "C"],
      ["add", null],
    ],
  );
  const lines = api.annotationsFor(uri).map((a) => [a.line, a.style, a.text]);
  assert.deepEqual(lines, [
    [5, "guess", "◌ increments c.n"],
    [6, "fact", "● calls add"],
    [9, "guess", "◌ returns the sum"],
  ]);

  // Hover comes from the server through VS Code's LSP client.
  const hovers = await vscode.commands.executeCommand<vscode.Hover[]>(
    "vscode.executeHoverProvider",
    doc.uri,
    new vscode.Position(5, 2),
  );
  const md = hovers
    .flatMap((h) => h.contents)
    .map((c) => (typeof c === "string" ? c : c.value))
    .join("\n");
  assert.match(md, /\*\*calls add\*\* _\(fact\)_/);

  // The first request for scenarios writes them; the view opens beside
  // the source.
  const quickPick = vscode.window.showQuickPick;
  (vscode.window as { showQuickPick: unknown }).showQuickPick = async (items: readonly unknown[]) => items[0];
  try {
    const editor = vscode.window.activeTextEditor!;
    editor.selection = new vscode.Selection(4, 0, 4, 0);
    await vscode.commands.executeCommand("codeReading.showScenario");
  } finally {
    (vscode.window as { showQuickPick: unknown }).showQuickPick = quickPick;
  }
  const scenarioDoc = await until(
    () => vscode.workspace.textDocuments.find((d) => d.uri.scheme === "crt-scenario"),
    5000,
    "the scenario document",
  );
  const text = scenarioDoc.getText();
  assert.match(text, /two Inc at once \(concurrent\)/);
  assert.match(text, /L6 {4}both read c\.n/);
  assert.match(text, /outcome: one increment is lost/);

  // The concurrent scenario is a diagnostic linking its steps.
  const diag = await until(
    () => vscode.languages.getDiagnostics(doc.uri)[0],
    10000,
    "the concurrency diagnostic",
  );
  assert.equal(diag.range.start.line, 5);
  assert.match(diag.message, /two Inc at once/);
  assert.equal(diag.relatedInformation?.[0]?.location.range.start.line, 6);

  // The status bar item turns explanations off and on; the choice is a
  // setting, so it survives a restart.
  await vscode.window.showTextDocument(doc);
  assert.match(api.statusText() ?? "", /^Code Reading: (on|writing…)$/);
  await vscode.commands.executeCommand("codeReading.toggle");
  await until(() => (api.statusText() === "Code Reading: off" ? true : undefined), 5000, "explanations off");
  assert.equal(vscode.workspace.getConfiguration("codeReading").get("enabled"), false);
  await vscode.commands.executeCommand("codeReading.toggle");
  await until(() => (api.statusText() === "Code Reading: on" ? true : undefined), 5000, "explanations on");

  // The configuration file opens from a command.
  await vscode.commands.executeCommand("codeReading.openConfig");
  await until(
    () => (vscode.window.activeTextEditor?.document.uri.fsPath.endsWith("config.toml") ? true : undefined),
    5000,
    "the configuration file",
  );

  // An explicit read is served from the cache.
  await vscode.window.showTextDocument(doc);
  await vscode.commands.executeCommand("codeReading.read");
}
