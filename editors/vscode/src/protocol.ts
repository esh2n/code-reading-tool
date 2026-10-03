// The JSON the crt server sends, mirroring crates/crt-wire. Line numbers
// inside these shapes are 1-based; LSP positions are 0-based.

export interface CallDto {
  name: string;
  defined_in_file: boolean;
}

export interface LineDto {
  line: number;
  calls: CallDto[];
}

export interface FunctionDto {
  name: string;
  kind: "function" | "method";
  enclosing: string | null;
  start_line: number;
  end_line: number;
  hash: string;
  docs: string | null;
  lines: LineDto[];
}

export interface FileAnalysisDto {
  wire_version: number;
  language: string;
  has_syntax_error: boolean;
  functions: FunctionDto[];
}

export type BasisDto = { kind: "fact"; call: string } | { kind: "inference" };

export interface NoteDto {
  line: number;
  text: string;
  detail: string | null;
  assumptions: string[];
  basis: BasisDto;
}

export interface StepDto {
  line: number;
  what: string;
}

export interface ScenarioDto {
  kind: "normal" | "boundary" | "concurrent";
  title: string;
  input: string;
  steps: StepDto[];
  outcome: string;
  assumptions: string[];
}

export interface ReadingDto {
  function_hash: string;
  model: string;
  prompt: string;
  notes: NoteDto[];
  /** `null` until scenarios are asked for (`codeReading/scenarios`). */
  scenarios: ScenarioDto[] | null;
  demoted: number;
  dropped: number;
}

export interface FileReadingsParams {
  uri: string;
  version: number;
  analysis: FileAnalysisDto;
  readings: (ReadingDto | null)[];
  pending: string[];
  /** Notes received so far for readings still being written. */
  partial?: PartialNotesDto[];
  /** The earlier reading of functions edited since, shown as old. */
  stale?: StaleReadingDto[];
}

export interface StaleReadingDto {
  functionHash: string;
  reading: ReadingDto;
}

export interface PartialNotesDto {
  functionHash: string;
  notes: NoteDto[];
}

export interface FunctionReadingDto {
  wire_version: number;
  language: string;
  has_syntax_error: boolean;
  from_cache: boolean;
  warnings: string[];
  function: FunctionDto;
  reading: ReadingDto;
}

export const FILE_READINGS = "codeReading/fileReadings";
export const READ = "codeReading/read";
export const SCENARIOS = "codeReading/scenarios";
export const CONFIG_PATH = "codeReading/configPath";

export interface ConfigPathResult {
  path: string;
  created: boolean;
}
export const VISIBLE_RANGE = "codeReading/visibleRange";
