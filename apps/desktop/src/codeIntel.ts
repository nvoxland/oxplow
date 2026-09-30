/**
 * Code intelligence read through the models (P6.E3, `.context/lsp.md`):
 * `v_diagnostic` for the Problems page and `v_symbol` for the Symbols
 * page and `symbol:` refs. Live questions (references, hover) stay with
 * the language server.
 */
import { querySql, type SqlCell } from "./api.js";
import type { Reads, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";

export type Severity = "error" | "warning" | "information" | "hint";

export interface Diagnostic {
  path: string;
  severity: Severity;
  message: string;
  source: string | null;
  code: string | null;
  line: number;
  col: number;
}

const at = (result: SqlQueryResult, row: SqlCell[], name: string) => row[result.columns.indexOf(name)];
const text = (v: SqlCell | undefined) => (v === null || v === undefined ? null : String(v));

export function diagnosticsFromResult(result: SqlQueryResult): Diagnostic[] {
  return result.rows.map((row) => ({
    path: String(at(result, row, "path")),
    severity: String(at(result, row, "severity")) as Severity,
    message: String(at(result, row, "message") ?? ""),
    source: text(at(result, row, "source")),
    code: text(at(result, row, "code")),
    line: Number(at(result, row, "line") ?? 1),
    col: Number(at(result, row, "col") ?? 1),
  }));
}

/** A stream's diagnostics (what its language servers report now). */
export async function readDiagnostics(streamRowId: number): Promise<{ diagnostics: Diagnostic[]; reads: Reads }> {
  const res = await querySql(
    `SELECT path, severity, message, source, code, line, col FROM v_diagnostic
      WHERE stream_id = ?1 ORDER BY path, line, col`,
    [streamRowId],
    10_000,
  );
  return { diagnostics: diagnosticsFromResult(res), reads: res.reads };
}

export interface FileProblems {
  path: string;
  errors: number;
  warnings: number;
  problems: Diagnostic[];
}

const RANK: Record<Severity, number> = { error: 0, warning: 1, information: 2, hint: 3 };

/** Diagnostics by file — files with errors first, then by count and path;
 *  each file's problems most severe first, then by line. `severity`
 *  keeps only that severity. */
export function problemsByFile(diagnostics: Diagnostic[], severity: Severity | null): FileProblems[] {
  const files = new Map<string, Diagnostic[]>();
  for (const d of diagnostics) {
    if (severity && d.severity !== severity) continue;
    files.set(d.path, [...(files.get(d.path) ?? []), d]);
  }
  return [...files.entries()]
    .map(([path, problems]) => ({
      path,
      errors: problems.filter((p) => p.severity === "error").length,
      warnings: problems.filter((p) => p.severity === "warning").length,
      problems: [...problems].sort((a, b) => RANK[a.severity] - RANK[b.severity] || a.line - b.line),
    }))
    .sort((a, b) => b.errors - a.errors || b.warnings - a.warnings || a.path.localeCompare(b.path));
}

export interface SymbolRow {
  ref: string;
  path: string;
  name: string;
  kind: string;
  /** The enclosing symbol path (`Widget`), or null at top level. */
  container: string | null;
  line: number;
  col: number;
}

export function symbolsFromResult(result: SqlQueryResult): SymbolRow[] {
  return result.rows.map((row) => ({
    ref: String(at(result, row, "ref")),
    path: String(at(result, row, "path")),
    name: String(at(result, row, "name")),
    kind: String(at(result, row, "kind") ?? ""),
    container: text(at(result, row, "container")),
    line: Number(at(result, row, "line") ?? 1),
    col: Number(at(result, row, "col") ?? 1),
  }));
}

/** A stream's symbols — one file's (`path`), or every file's matching
 *  `filter` (a name substring) — in file order. */
export async function readSymbols(
  streamRowId: number,
  opts: { path?: string | null; filter?: string },
): Promise<{ symbols: SymbolRow[]; reads: Reads }> {
  const where = ["stream_id = ?1"];
  const params: SqlCell[] = [streamRowId];
  if (opts.path) {
    params.push(opts.path);
    where.push(`path = ?${params.length}`);
  }
  if (opts.filter?.trim()) {
    params.push(`%${opts.filter.trim()}%`);
    where.push(`name LIKE ?${params.length}`);
  }
  const res = await querySql(
    `SELECT ref, path, name, kind, container, line, col FROM v_symbol
      WHERE ${where.join(" AND ")} ORDER BY path, line, col`,
    params,
    5_000,
  );
  return { symbols: symbolsFromResult(res), reads: res.reads };
}

export interface SymbolNode {
  symbol: SymbolRow;
  children: SymbolNode[];
}

/** Symbols nested under their container (per file), in file order. A
 *  symbol whose container isn't in the list sits at the top. */
export function symbolTree(symbols: SymbolRow[]): SymbolNode[] {
  const byPath = new Map<string, SymbolNode>();
  const roots: SymbolNode[] = [];
  const key = (path: string, namePath: string) => `${path}\u0000${namePath}`;
  for (const s of symbols) {
    const node: SymbolNode = { symbol: s, children: [] };
    const own = s.container ? `${s.container}::${s.name}` : s.name;
    byPath.set(key(s.path, own), node);
    const parent = s.container ? byPath.get(key(s.path, s.container)) : undefined;
    (parent ? parent.children : roots).push(node);
  }
  return roots;
}

export function symbolLocation(result: SqlQueryResult): { path: string; line: number; col: number } | null {
  const row = result.rows[0];
  if (!row) return null;
  return {
    path: String(at(result, row, "path")),
    line: Number(at(result, row, "line") ?? 1),
    col: Number(at(result, row, "col") ?? 1),
  };
}

/** Where a `symbol:` ref's name is, or null when it isn't a symbol now. */
export async function resolveSymbol(ref: string): Promise<{ path: string; line: number; col: number } | null> {
  return symbolLocation(await querySql("SELECT path, line, col FROM v_symbol WHERE ref = ?1", [ref], 1));
}
