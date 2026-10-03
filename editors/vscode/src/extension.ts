// Starts the crt language server, renders what it pushes, reports the
// visible ranges, and offers the scenario view. Hover and diagnostics come
// through VS Code's own LSP client.

import * as fs from "node:fs";
import * as path from "node:path";
import * as vscode from "vscode";
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from "vscode-languageclient/node";

import { type Annotation, type Style, annotations, visible } from "./annotations";
import {
  FILE_READINGS,
  type FileReadingsParams,
  type FunctionDto,
  type FunctionReadingDto,
  READ,
  type ReadingDto,
  CONFIG_PATH,
  type ConfigPathResult,
  SCENARIOS,
  VISIBLE_RANGE,
} from "./protocol";
import { scenarioLabel, scenarioText } from "./scenario";

const SELECTOR = [
  "rust",
  "go",
  "python",
  "javascript",
  "javascriptreact",
  "typescript",
  "typescriptreact",
  "java",
  "c",
  "cpp",
  "csharp",
  "ruby",
  "php",
].map((language) => ({
  scheme: "file",
  language,
}));
const BUFFER_LINES = 50;
const SCENARIO_SCHEME = "crt-scenario";

let client: LanguageClient | undefined;
const latest = new Map<string, FileReadingsParams>();
/** `codeReading.enabled`: annotations shown and auto-read on. */
let enabled = true;
let status: vscode.StatusBarItem | undefined;
/** The last diagnostics the server sent for each document. */
const serverDiagnostics = new Map<string, vscode.Diagnostic[]>();

/** Shows or hides the server's diagnostics to match `enabled`. */
function applyDiagnostics(): void {
  const collection = client?.diagnostics;
  if (!collection) return;
  for (const [uri, diagnostics] of serverDiagnostics) {
    collection.set(vscode.Uri.parse(uri), enabled ? diagnostics : []);
  }
}

function readEnabled(): boolean {
  return vscode.workspace.getConfiguration("codeReading").get<boolean>("enabled", true);
}

/** The status bar item: whether explanations are on, and whether some are being written. */
function updateStatus(): void {
  if (!status) return;
  const doc = vscode.window.activeTextEditor?.document;
  if (!doc || vscode.languages.match(SELECTOR, doc) === 0) {
    status.hide();
    return;
  }
  const writing = (latest.get(doc.uri.toString())?.pending.length ?? 0) > 0;
  if (!enabled) {
    status.text = "Code Reading: off";
    status.tooltip = "Explanations are off. Click to turn them on.";
  } else if (writing) {
    status.text = "Code Reading: writing…";
    status.tooltip = "Writing explanations for the code in view. Click to turn explanations off.";
  } else {
    status.text = "Code Reading: on";
    status.tooltip = "Explanations are on. Click to turn them off.";
  }
  status.show();
}

const styles: Record<Style, vscode.TextEditorDecorationType> = {} as Record<
  Style,
  vscode.TextEditorDecorationType
>;

function makeStyles(): void {
  const make = (color: string, italic: boolean) =>
    vscode.window.createTextEditorDecorationType({
      after: { margin: "0 0 0 2em", color: new vscode.ThemeColor(color), fontStyle: italic ? "italic" : "normal" },
      rangeBehavior: vscode.DecorationRangeBehavior.ClosedClosed,
    });
  styles.fact = make("testing.iconPassed", false);
  styles.guess = make("editorCodeLens.foreground", true);
  styles.call = make("editorLineNumber.foreground", false);
  styles.pending = make("editorInfo.foreground", true);
}

/** The server binary: setting, then bundled, then `crt` on PATH. */
function serverPath(context: vscode.ExtensionContext): string {
  const configured = vscode.workspace.getConfiguration("codeReading").get<string>("serverPath");
  if (configured) {
    return configured;
  }
  const exe = process.platform === "win32" ? "crt.exe" : "crt";
  const bundled = path.join(context.extensionPath, "bin", exe);
  return fs.existsSync(bundled) ? bundled : "crt";
}

function render(editor: vscode.TextEditor): void {
  const params = latest.get(editor.document.uri.toString());
  const by: Record<Style, vscode.DecorationOptions[]> = { fact: [], guess: [], call: [], pending: [] };
  if (params && enabled && params.version >= 0) {
    const ranges = editor.visibleRanges.map((r) => ({ start: r.start.line + 1, end: r.end.line + 1 }));
    const lines = editor.document.lineCount;
    for (const a of visible(annotations(params), ranges, BUFFER_LINES)) {
      if (a.line > lines) continue;
      const end = editor.document.lineAt(a.line - 1).range.end;
      by[a.style].push({ range: new vscode.Range(end, end), renderOptions: { after: { contentText: a.text } } });
    }
  }
  for (const style of Object.keys(by) as Style[]) {
    editor.setDecorations(styles[style], by[style]);
  }
}

function renderAll(): void {
  for (const editor of vscode.window.visibleTextEditors) {
    render(editor);
  }
}

const viewTimers = new Map<string, NodeJS.Timeout>();

function reportView(editor: vscode.TextEditor): void {
  // Off means no model calls unless asked: the server reads only what it
  // is told is in view.
  if (!enabled) return;
  const key = editor.document.uri.toString();
  clearTimeout(viewTimers.get(key));
  viewTimers.set(
    key,
    setTimeout(() => {
      const ranges = editor.visibleRanges;
      const first = ranges[0];
      const last = ranges[ranges.length - 1];
      if (!client || !first || !last) return;
      void client.sendNotification(VISIBLE_RANGE, {
        uri: key,
        startLine: first.start.line,
        endLine: last.end.line,
      });
    }, 150),
  );
}

/** The innermost function containing the 1-based line, with its reading. */
function functionAt(uri: string, line: number): { fn: FunctionDto; reading: ReadingDto | null } | undefined {
  const params = latest.get(uri);
  if (!params) return undefined;
  let best: { fn: FunctionDto; reading: ReadingDto | null } | undefined;
  params.analysis.functions.forEach((fn, i) => {
    if (fn.start_line <= line && line <= fn.end_line) {
      if (!best || fn.end_line - fn.start_line < best.fn.end_line - best.fn.start_line) {
        best = { fn, reading: params.readings[i] ?? null };
      }
    }
  });
  return best;
}

async function read(refresh: boolean): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor || !client) return;
  const result = await vscode.window.withProgress(
    { location: vscode.ProgressLocation.Window, title: "Code Reading: explaining…" },
    () =>
      client!.sendRequest<FunctionReadingDto>(READ, {
        uri: editor.document.uri.toString(),
        line: editor.selection.active.line,
        refresh,
      }),
  );
  for (const w of result.warnings) {
    void vscode.window.showWarningMessage(`Code Reading: ${w}`);
  }
}

const scenarioDocs = new Map<string, string>();
const stepStyle = vscode.window.createTextEditorDecorationType({
  isWholeLine: true,
  backgroundColor: new vscode.ThemeColor("editor.rangeHighlightBackground"),
});

async function showScenario(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) return;
  const found = functionAt(editor.document.uri.toString(), editor.selection.active.line + 1);
  if (!found) {
    void vscode.window.showInformationMessage("Code Reading: no function under the cursor.");
    return;
  }
  let scenarios = found.reading?.scenarios ?? null;
  if (scenarios === null) {
    // Written on first use (one model call), then cached with the notes.
    if (!client) return;
    const c = client;
    const result = await vscode.window.withProgress(
      { location: vscode.ProgressLocation.Window, title: `Code Reading: writing scenarios for ${found.fn.name}…` },
      () =>
        c.sendRequest<FunctionReadingDto>(SCENARIOS, {
          uri: editor.document.uri.toString(),
          line: found.fn.start_line - 1,
          refresh: false,
        }),
    );
    for (const w of result.warnings) {
      void vscode.window.showWarningMessage(`Code Reading: ${w}`);
    }
    scenarios = result.reading.scenarios ?? [];
  }
  if (scenarios.length === 0) {
    void vscode.window.showInformationMessage(`Code Reading: no scenarios for ${found.fn.name}.`);
    return;
  }
  const pick = await vscode.window.showQuickPick(
    scenarios.map((sc) => ({ label: scenarioLabel(sc), sc })),
    { placeHolder: `Scenario for ${found.fn.name}` },
  );
  if (!pick) return;
  editor.setDecorations(
    stepStyle,
    pick.sc.steps.map((s) => new vscode.Range(s.line - 1, 0, s.line - 1, 0)),
  );
  const uri = vscode.Uri.from({
    scheme: SCENARIO_SCHEME,
    path: `/${found.fn.name}/${pick.sc.title.replace(/[\\/]/g, "_")}.txt`,
  });
  scenarioDocs.set(uri.toString(), scenarioText(found.fn, pick.sc));
  const doc = await vscode.workspace.openTextDocument(uri);
  await vscode.window.showTextDocument(doc, { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true });
}

/**
 * Opens crt's configuration file: the endpoint, model, key and output
 * language every editor shares. The server writes a commented example when
 * there is none; changes apply on the next read.
 */
async function openConfig(): Promise<void> {
  if (!client) return;
  const result = await client.sendRequest<ConfigPathResult>(CONFIG_PATH);
  const doc = await vscode.workspace.openTextDocument(vscode.Uri.file(result.path));
  await vscode.window.showTextDocument(doc);
  if (result.created) {
    void vscode.window.showInformationMessage("Code Reading: wrote an example configuration; edit it to choose a model.");
  }
}

/** For tests: what this extension last rendered for a document. */
export interface TestApi {
  /** The status bar item's text. */
  statusText(): string | undefined;
  annotationsFor(uri: string): Annotation[];
  /** True once notes still being written were received for `uri`. */
  sawPartial(uri: string): boolean;
  waitForReadings(uri: string, timeoutMs: number): Promise<FileReadingsParams>;
}

export async function activate(context: vscode.ExtensionContext): Promise<TestApi> {
  makeStyles();
  enabled = readEnabled();
  status = vscode.window.createStatusBarItem("codeReading.status", vscode.StatusBarAlignment.Right, 100);
  status.name = "Code Reading";
  status.command = "codeReading.toggle";
  const settings = vscode.workspace.getConfiguration("codeReading");
  const args = ["lsp"];
  const configPath = settings.get<string>("configPath");
  if (configPath) args.push("--config", configPath);
  const cacheDir = settings.get<string>("cacheDir");
  if (cacheDir) args.push("--cache-dir", cacheDir);
  const server: ServerOptions = { command: serverPath(context), args };
  const options: LanguageClientOptions = {
    documentSelector: SELECTOR,
    initializationOptions: {
      autoRead: settings.get<boolean>("autoRead", true),
      maxParallel: settings.get<number>("maxParallel", 2),
    },
    middleware: {
      // The server's diagnostics (concurrency scenarios) are kept, and shown
      // only while explanations are on.
      handleDiagnostics: (uri, diagnostics, next) => {
        serverDiagnostics.set(uri.toString(), diagnostics);
        next(uri, enabled ? diagnostics : []);
      },
    },
  };
  client = new LanguageClient("crt", "Code Reading", server, options);

  const waiters: { uri: string; done: (p: FileReadingsParams) => boolean }[] = [];
  const partialSeen = new Set<string>();
  client.onNotification(FILE_READINGS, (params: FileReadingsParams) => {
    latest.set(params.uri, params);
    if ((params.partial ?? []).some((p) => p.notes.length > 0)) partialSeen.add(params.uri);
    for (const editor of vscode.window.visibleTextEditors) {
      if (editor.document.uri.toString() === params.uri) render(editor);
    }
    updateStatus();
    for (let i = waiters.length - 1; i >= 0; i--) {
      const w = waiters[i]!;
      if (w.uri === params.uri && w.done(params)) waiters.splice(i, 1);
    }
  });

  context.subscriptions.push(
    vscode.workspace.registerTextDocumentContentProvider(SCENARIO_SCHEME, {
      provideTextDocumentContent: (uri) => scenarioDocs.get(uri.toString()) ?? "",
    }),
    vscode.window.onDidChangeTextEditorVisibleRanges((e) => {
      render(e.textEditor);
      reportView(e.textEditor);
    }),
    vscode.window.onDidChangeVisibleTextEditors((editors) => {
      for (const e of editors) {
        render(e);
        reportView(e);
      }
    }),
    vscode.commands.registerCommand("codeReading.read", () => read(false)),
    vscode.commands.registerCommand("codeReading.refresh", () => read(true)),
    vscode.commands.registerCommand("codeReading.showScenario", showScenario),
    vscode.commands.registerCommand("codeReading.openConfig", openConfig),
    // Kept in the user's settings, so the choice survives a restart.
    vscode.commands.registerCommand("codeReading.toggle", () =>
      vscode.workspace
        .getConfiguration("codeReading")
        .update("enabled", !enabled, vscode.ConfigurationTarget.Global),
    ),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (!e.affectsConfiguration("codeReading.enabled")) return;
      enabled = readEnabled();
      renderAll();
      applyDiagnostics();
      updateStatus();
      if (enabled) for (const editor of vscode.window.visibleTextEditors) reportView(editor);
    }),
    vscode.window.onDidChangeActiveTextEditor(() => updateStatus()),
    status,
    ...Object.values(styles),
    stepStyle,
  );

  await client.start();
  for (const e of vscode.window.visibleTextEditors) reportView(e);
  updateStatus();

  return {
    sawPartial: (uri) => partialSeen.has(uri),
    statusText: () => status?.text,
    annotationsFor: (uri) => {
      const p = latest.get(uri);
      return p ? annotations(p) : [];
    },
    waitForReadings: (uri, timeoutMs) =>
      new Promise((resolve, reject) => {
        const complete = (p: FileReadingsParams) => p.readings.length > 0 && p.readings.every((r) => r !== null);
        const now = latest.get(uri);
        if (now && complete(now)) return resolve(now);
        const timer = setTimeout(() => reject(new Error("timed out waiting for readings")), timeoutMs);
        waiters.push({
          uri,
          done: (p) => {
            if (!complete(p)) return false;
            clearTimeout(timer);
            resolve(p);
            return true;
          },
        });
      }),
  };
}

export async function deactivate(): Promise<void> {
  await client?.stop();
}
