// Launches a downloaded VS Code with this extension against `crt lsp` and a
// scripted OpenAI-compatible server, then runs suite.ts inside it.

import * as fs from "node:fs";
import * as http from "node:http";
import * as os from "node:os";
import * as path from "node:path";
import { runTests } from "@vscode/test-electron";

const SOURCE =
  "package p\n\ntype C struct{ n int }\n\nfunc (c *C) Inc() {\n\tc.n = add(c.n, 1)\n}\n\nfunc add(a, b int) int { return a + b }\n";

/**
 * The answer to one request as the pieces of a streamed response; `null`
 * is a pause. The notes of Inc pause after the first note, so the editor
 * shows it on its own before the second arrives.
 */
function answerFor(task: string, user: string): (string | null)[] {
  if (task === "function_scenarios") {
    const content = {
      scenarios: [
        {
          kind: "concurrent",
          title: "two Inc at once",
          input: "two goroutines",
          steps: [
            { line: 6, what: "both read c.n" },
            { line: 7, what: "both return" },
          ],
          outcome: "one increment is lost",
          assumptions: ["no lock"],
        },
      ],
    };
    return [JSON.stringify(content)];
  }
  if (user.includes("FUNCTION: Inc")) {
    const first = { line: 5, text: "increments c.n", detail: "", assumptions: [], basis: { kind: "inference", call: null } };
    const second = { line: 6, text: "calls add", detail: "adds one", assumptions: [], basis: { kind: "fact", call: "add" } };
    return [`{"notes":[${JSON.stringify(first)},`, null, `${JSON.stringify(second)}]}`];
  }
  const content = {
    notes: [{ line: 9, text: "returns the sum", detail: "", assumptions: [], basis: { kind: "inference", call: null } }],
  };
  return [JSON.stringify(content)];
}

function serveLlm(): Promise<{ url: string; close: () => void }> {
  const server = http.createServer((req, res) => {
    let body = "";
    req.on("data", (c: Buffer) => (body += c.toString()));
    req.on("end", () => {
      const parsed = JSON.parse(body) as {
        messages: { content: string }[];
        response_format: { json_schema: { name: string } };
      };
      const pieces = answerFor(parsed.response_format.json_schema.name, parsed.messages[1]?.content ?? "");
      res.setHeader("content-type", "text/event-stream");
      void (async () => {
        for (const piece of pieces) {
          if (piece === null) {
            await new Promise((r) => setTimeout(r, 600));
            continue;
          }
          res.write(`data: ${JSON.stringify({ choices: [{ delta: { content: piece }, finish_reason: null }] })}\n\n`);
        }
        res.write(`data: ${JSON.stringify({ choices: [{ delta: {}, finish_reason: "stop" }] })}\n\n`);
        res.end("data: [DONE]\n\n");
      })();
    });
  });
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      const addr = server.address() as { port: number };
      resolve({ url: `http://127.0.0.1:${addr.port}/v1`, close: () => server.close() });
    });
  });
}

async function main(): Promise<void> {
  const root = path.resolve(__dirname, "../..");
  const crt = process.env.CRT_BIN ?? path.resolve(root, "../../target/debug/crt");
  if (!fs.existsSync(crt)) {
    throw new Error(`crt binary not found at ${crt}; build it with cargo build or set CRT_BIN`);
  }
  const llm = await serveLlm();
  const work = fs.mkdtempSync(path.join(os.tmpdir(), "crt-vscode-"));
  const workspace = path.join(work, "ws");
  fs.mkdirSync(path.join(workspace, ".vscode"), { recursive: true });
  fs.writeFileSync(path.join(workspace, "p.go"), SOURCE);
  const config = path.join(work, "config.toml");
  fs.writeFileSync(config, `[llm]\nbase_url = "${llm.url}"\nmodel = "m"\n`);
  // Machine-scoped settings are ignored in workspace settings by design, so
  // they go in the test profile's user settings.
  const userDir = path.join(work, "user");
  fs.mkdirSync(path.join(userDir, "User"), { recursive: true });
  fs.writeFileSync(
    path.join(userDir, "User", "settings.json"),
    JSON.stringify({
      "codeReading.serverPath": crt,
      "codeReading.configPath": config,
      "codeReading.cacheDir": path.join(work, "cache"),
      "security.workspace.trust.enabled": false,
    }),
  );
  try {
    await runTests({
      extensionDevelopmentPath: root,
      extensionTestsPath: path.join(__dirname, "suite.js"),
      launchArgs: [workspace, "--disable-extensions", "--user-data-dir", userDir],
      extensionTestsEnv: { CRT_TEST_FILE: path.join(workspace, "p.go") },
    });
  } finally {
    llm.close();
  }
}

main().catch((e: unknown) => {
  console.error(e);
  process.exit(1);
});
