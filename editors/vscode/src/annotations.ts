// What to show at the end of each line. Pure: no VS Code API, so it is unit
// tested and shared by the renderer.

import type { FileReadingsParams, FunctionDto, NoteDto } from "./protocol";

export type Style = "fact" | "guess" | "call" | "pending";

export interface Annotation {
  /** 1-based line. */
  line: number;
  text: string;
  style: Style;
}

const MARK: Record<"fact" | "inference", string> = { fact: "● ", inference: "◌ " };

/** Every annotated line of the document, in line order. */
export function annotations(params: FileReadingsParams): Annotation[] {
  const pending = new Set(params.pending);
  // Notes still arriving stand in for the reading until it is finished.
  const partial = new Map((params.partial ?? []).map((p) => [p.functionHash, p.notes]));
  const out: Annotation[] = [];
  params.analysis.functions.forEach((fn, i) => {
    const notes = partial.get(fn.hash) ?? params.readings[i]?.notes ?? [];
    out.push(...forFunction(fn, notes, pending.has(fn.hash)));
  });
  return out.sort((a, b) => a.line - b.line);
}

function forFunction(fn: FunctionDto, all: NoteDto[], pending: boolean): Annotation[] {
  const out: Annotation[] = [];
  for (let line = fn.start_line; line <= fn.end_line; line++) {
    const notes = all.filter((n) => n.line === line);
    const first = notes[0];
    if (first) {
      const more = notes.length > 1 ? ` +${notes.length - 1}` : "";
      out.push({
        line,
        text: MARK[first.basis.kind] + first.text + more,
        style: first.basis.kind === "fact" ? "fact" : "guess",
      });
    } else {
      const calls = fn.lines.find((l) => l.line === line)?.calls ?? [];
      if (calls.length > 0) {
        out.push({ line, text: "→ " + calls.map((c) => c.name).join(", "), style: "call" });
      }
    }
    if (line === fn.start_line && pending) {
      out.push({ line, text: "⋯ reading", style: "pending" });
    }
  }
  return out;
}

/**
 * The annotations to render for the visible ranges, widened by `buffer`
 * lines. VS Code creates CSS rules per distinct text, so rendering a whole
 * large file at once can freeze the editor; this keeps it to what is seen.
 */
export function visible(
  all: Annotation[],
  ranges: { start: number; end: number }[],
  buffer: number,
): Annotation[] {
  return all.filter((a) => ranges.some((r) => a.line >= r.start - buffer && a.line <= r.end + buffer));
}
