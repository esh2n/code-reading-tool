import assert from "node:assert/strict";
import { test } from "node:test";

import { annotations, visible } from "../../src/annotations";
import type { FileReadingsParams } from "../../src/protocol";
import { scenarioText } from "../../src/scenario";

const params: FileReadingsParams = {
  uri: "file:///a.go",
  version: 1,
  analysis: {
    wire_version: 1,
    language: "go",
    has_syntax_error: false,
    functions: [
      {
        name: "Inc",
        kind: "method",
        enclosing: "C",
        start_line: 5,
        end_line: 8,
        hash: "h1",
        docs: null,
        lines: [
          { line: 6, calls: [{ name: "add", defined_in_file: true }] },
          { line: 7, calls: [{ name: "log", defined_in_file: false }, { name: "x", defined_in_file: false }] },
        ],
      },
      {
        name: "add",
        kind: "function",
        enclosing: null,
        start_line: 10,
        end_line: 10,
        hash: "h2",
        docs: null,
        lines: [],
      },
    ],
  },
  readings: [
    {
      function_hash: "h1",
      model: "m",
      prompt: "v1-english",
      notes: [
        { line: 6, text: "calls add", detail: null, assumptions: [], basis: { kind: "fact", call: "add" } },
        { line: 6, text: "second", detail: null, assumptions: [], basis: { kind: "inference" } },
        { line: 8, text: "returns", detail: null, assumptions: [], basis: { kind: "inference" } },
      ],
      scenarios: [],
      demoted: 0,
      dropped: 0,
    },
    null,
  ],
  pending: ["h2"],
  partial: [],
};

test("notes win over call facts, guesses and facts are styled apart", () => {
  const a = annotations(params);
  assert.deepEqual(
    a.map((x) => [x.line, x.style, x.text]),
    [
      [6, "fact", "● calls add +1"],
      [7, "call", "→ log, x"],
      [8, "guess", "◌ returns"],
      [10, "pending", "⋯ reading"],
    ],
  );
});

test("notes still arriving stand in for the reading", () => {
  const a = annotations({
    ...params,
    partial: [
      {
        functionHash: "h2",
        notes: [{ line: 10, text: "adds", detail: null, assumptions: [], basis: { kind: "inference" } }],
      },
    ],
  });
  assert.deepEqual(
    a.filter((x) => x.line === 10).map((x) => [x.style, x.text]),
    [
      ["guess", "◌ adds"],
      ["pending", "⋯ reading"],
    ],
  );
});

test("an edited function shows its earlier notes, marked old", () => {
  const a = annotations({
    ...params,
    pending: [],
    stale: [
      {
        functionHash: "h2",
        reading: {
          function_hash: "old",
          model: "m",
          prompt: "p",
          notes: [{ line: 10, text: "adds", detail: null, assumptions: [], basis: { kind: "fact", call: "x" } }],
          scenarios: null,
          demoted: 0,
          dropped: 0,
        },
      },
    ],
  });
  assert.deepEqual(
    a.filter((x) => x.line === 10).map((x) => [x.style, x.text]),
    [["stale", "(old) adds"]],
  );
});

test("only lines near the visible range are rendered", () => {
  const a = annotations(params);
  assert.deepEqual(
    visible(a, [{ start: 1, end: 6 }], 1).map((x) => x.line),
    [6, 7],
  );
  assert.equal(visible(a, [{ start: 100, end: 120 }], 50).length, 0);
});

test("scenario text lists input, steps and outcome, and says it is a guess", () => {
  const text = scenarioText(params.analysis.functions[0]!, {
    kind: "concurrent",
    title: "two at once",
    input: "two goroutines",
    steps: [
      { line: 6, what: "both read" },
      { line: 7, what: "both write" },
    ],
    outcome: "lost update",
    assumptions: ["no lock"],
  });
  assert.match(text, /^method Inc — two at once \(concurrent\)/);
  assert.match(text, /guess: reasoned from the code, not executed/);
  assert.match(text, /\n {2}L6 {4}both read\n/);
  assert.match(text, /outcome: lost update/);
  assert.match(text, /assumes:\n {2}- no lock/);
});
