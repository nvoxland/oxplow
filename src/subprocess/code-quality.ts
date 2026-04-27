import { execFile } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { promisify } from "node:util";
import type { CodeQualityFinding } from "../persistence/code-quality-store.js";

const execFileAsync = promisify(execFile);

export interface RunCodeQualityOptions {
  /** Subset of repo-relative paths to scan. If omitted, scans the whole projectDir. */
  files?: string[];
  /** Subprocess timeout in ms. Defaults to 60_000. */
  timeoutMs?: number;
}

export class CodeQualityToolMissingError extends Error {
  constructor(public readonly tool: string) {
    super(`${tool} not found on PATH`);
    this.name = "CodeQualityToolMissingError";
  }
}

/**
 * Run lizard against the project (or a file subset). Returns one finding
 * per function, with `kind` set to `complexity`, `function-length`, or
 * `parameter-count` per the metric and `metric_value` set to the numeric
 * value. The same function emits up to three findings — the UI groups
 * by function via `extra.functionName`.
 */
export async function runLizard(
  projectDir: string,
  options: RunCodeQualityOptions = {},
): Promise<CodeQualityFinding[]> {
  const args = ["--csv"];
  if (options.files && options.files.length > 0) {
    args.push(...options.files);
  } else {
    args.push(projectDir);
  }
  let stdout: string;
  try {
    const result = await execFileAsync("lizard", args, {
      cwd: projectDir,
      timeout: options.timeoutMs ?? 60_000,
      maxBuffer: 64 * 1024 * 1024,
    });
    stdout = result.stdout;
  } catch (err) {
    throw mapSpawnError(err, "lizard");
  }
  return parseLizardCsv(stdout, projectDir);
}

/**
 * Run jscpd against the project (or a file subset). Returns one finding
 * per duplicate-block instance with `kind = 'duplicate-block'` and
 * `metric_value` set to the duplicated-line count. `extra` carries the
 * peer location so the UI can show "duplicates X lines from Y:Lstart-Lend".
 */
export async function runJscpd(
  projectDir: string,
  options: RunCodeQualityOptions = {},
): Promise<CodeQualityFinding[]> {
  const outDir = mkdtempSync(join(tmpdir(), "oxplow-jscpd-"));
  try {
    const args = ["--reporters", "json", "--silent", "--output", outDir];
    if (options.files && options.files.length > 0) {
      // jscpd accepts comma-separated patterns; pass file list directly.
      args.push("--pattern", options.files.join(","));
    }
    args.push(projectDir);
    try {
      await execFileAsync("jscpd", args, {
        cwd: projectDir,
        timeout: options.timeoutMs ?? 60_000,
        maxBuffer: 64 * 1024 * 1024,
      });
    } catch (err) {
      throw mapSpawnError(err, "jscpd");
    }
    const reportPath = join(outDir, "jscpd-report.json");
    let raw: string;
    try {
      raw = readFileSync(reportPath, "utf8");
    } catch {
      // jscpd writes no report when zero duplicates are found in some
      // versions; treat that as no findings.
      return [];
    }
    return parseJscpdReport(raw, projectDir);
  } finally {
    try {
      rmSync(outDir, { recursive: true, force: true });
    } catch {
      /* best-effort cleanup */
    }
  }
}

/**
 * Parse lizard's CSV output into normalized findings. Format per row:
 * `NLOC,CCN,token,PARAM,length,location`. `location` looks like
 * `name@start-end@filename`. We emit up to three findings per function:
 * one for complexity, one for function length, one for parameter count.
 *
 * Exported for tests (so we can verify parsing without lizard installed).
 */
export function parseLizardCsv(csv: string, projectDir: string): CodeQualityFinding[] {
  const out: CodeQualityFinding[] = [];
  const lines = csv.split(/\r?\n/);
  for (const line of lines) {
    const trimmed = line.trim();
    if (!trimmed) continue;
    const cols = splitCsvLine(trimmed);
    if (cols.length < 6) continue;
    const nloc = Number(cols[0]);
    const ccn = Number(cols[1]);
    const params = Number(cols[3]);
    const length = Number(cols[4]);
    const location = cols[5]!;
    const parsed = parseLizardLocation(location);
    if (!parsed) continue;
    if (!Number.isFinite(ccn) || !Number.isFinite(length) || !Number.isFinite(params)) continue;
    const path = toRepoRelative(parsed.file, projectDir);
    const extra = { functionName: parsed.name, nloc };
    out.push({
      path,
      startLine: parsed.startLine,
      endLine: parsed.endLine,
      kind: "complexity",
      metricValue: ccn,
      extra,
    });
    out.push({
      path,
      startLine: parsed.startLine,
      endLine: parsed.endLine,
      kind: "function-length",
      metricValue: length,
      extra,
    });
    out.push({
      path,
      startLine: parsed.startLine,
      endLine: parsed.endLine,
      kind: "parameter-count",
      metricValue: params,
      extra,
    });
  }
  return out;
}

interface JscpdReport {
  duplicates?: Array<{
    firstFile: { name: string; start: number; end: number };
    secondFile: { name: string; start: number; end: number };
    lines?: number;
  }>;
}

/**
 * Parse jscpd's `jscpd-report.json` into normalized findings. Each
 * duplicate-pair becomes two findings (one per side) so a click on
 * either side jumps the user to the right place; the peer location is
 * stashed in `extra` so the UI can render "duplicates X lines from
 * Y:Lstart-Lend".
 *
 * Exported for tests.
 */
export function parseJscpdReport(json: string, projectDir: string): CodeQualityFinding[] {
  let report: JscpdReport;
  try {
    report = JSON.parse(json) as JscpdReport;
  } catch {
    return [];
  }
  const out: CodeQualityFinding[] = [];
  for (const dup of report.duplicates ?? []) {
    const lines = dup.lines ?? Math.max(1, dup.firstFile.end - dup.firstFile.start + 1);
    const firstPath = toRepoRelative(dup.firstFile.name, projectDir);
    const secondPath = toRepoRelative(dup.secondFile.name, projectDir);
    out.push({
      path: firstPath,
      startLine: dup.firstFile.start,
      endLine: dup.firstFile.end,
      kind: "duplicate-block",
      metricValue: lines,
      extra: {
        peerPath: secondPath,
        peerStartLine: dup.secondFile.start,
        peerEndLine: dup.secondFile.end,
      },
    });
    out.push({
      path: secondPath,
      startLine: dup.secondFile.start,
      endLine: dup.secondFile.end,
      kind: "duplicate-block",
      metricValue: lines,
      extra: {
        peerPath: firstPath,
        peerStartLine: dup.firstFile.start,
        peerEndLine: dup.firstFile.end,
      },
    });
  }
  return out;
}

function splitCsvLine(line: string): string[] {
  const out: string[] = [];
  let buf = "";
  let inQuote = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (inQuote) {
      if (ch === '"') {
        if (line[i + 1] === '"') {
          buf += '"';
          i++;
        } else {
          inQuote = false;
        }
      } else {
        buf += ch;
      }
    } else {
      if (ch === '"') inQuote = true;
      else if (ch === ",") {
        out.push(buf);
        buf = "";
      } else buf += ch;
    }
  }
  out.push(buf);
  return out;
}

function parseLizardLocation(
  location: string,
): { name: string; startLine: number; endLine: number; file: string } | null {
  // `name@start-end@filename` (lizard's --csv format).
  const parts = location.split("@");
  if (parts.length < 3) return null;
  const name = parts[0]!;
  const range = parts[1]!;
  const file = parts.slice(2).join("@");
  const [startStr, endStr] = range.split("-");
  const startLine = Number(startStr);
  const endLine = Number(endStr);
  if (!Number.isFinite(startLine) || !Number.isFinite(endLine)) return null;
  return { name, startLine, endLine, file };
}

function toRepoRelative(absOrRel: string, projectDir: string): string {
  if (!absOrRel) return absOrRel;
  if (absOrRel.startsWith(projectDir)) {
    const rel = relative(projectDir, absOrRel);
    return rel || absOrRel;
  }
  return absOrRel;
}

function mapSpawnError(err: unknown, tool: string): Error {
  const e = err as NodeJS.ErrnoException & { code?: string };
  if (e?.code === "ENOENT") return new CodeQualityToolMissingError(tool);
  return err instanceof Error ? err : new Error(String(err));
}
