// The scenario document's text. Pure, so it is unit tested.

import type { FunctionDto, ScenarioDto } from "./protocol";

export function scenarioText(fn: FunctionDto, sc: ScenarioDto): string {
  const rows = [
    `${fn.kind} ${fn.name} — ${sc.title} (${sc.kind})`,
    "guess: reasoned from the code, not executed",
    "",
    `input:   ${sc.input}`,
    "",
    ...sc.steps.map((s) => `  L${String(s.line).padEnd(4)} ${s.what}`),
    "",
    `outcome: ${sc.outcome}`,
  ];
  if (sc.assumptions.length > 0) {
    rows.push("assumes:", ...sc.assumptions.map((a) => `  - ${a}`));
  }
  return rows.join("\n") + "\n";
}

export function scenarioLabel(sc: ScenarioDto): string {
  return `[${sc.kind}] ${sc.title}`;
}
