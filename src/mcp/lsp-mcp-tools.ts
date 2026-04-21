import { readFileSync } from "node:fs";
import { isAbsolute, relative, resolve } from "node:path";
import type { ToolDef } from "./mcp-server.js";
import { fileUri, lspLanguageIdForPath, type LspDiagnostic, type LspSession } from "../lsp/lsp.js";
import type { Stream } from "../persistence/stream-store.js";

const DIAGNOSTICS_WAIT_MS = 2000;

export interface LspManagerLike {
  getSession(stream: Stream, languageId: string): Promise<LspSession>;
}

export interface LspMcpToolDeps {
  resolveStream(streamId: string | undefined): Stream;
  lspManager: LspManagerLike;
}

interface BaseArgs {
  streamId?: string;
  path: string;
}

interface PositionArgs extends BaseArgs {
  line: number;
  column: number;
}

interface ResolvedDoc {
  stream: Stream;
  absolutePath: string;
  relativePath: string;
  uri: string;
  session: LspSession;
  text: string;
}

async function resolveAndOpen(
  deps: LspMcpToolDeps,
  args: BaseArgs,
): Promise<ResolvedDoc> {
  const stream = deps.resolveStream(args.streamId);
  if (!args.path || typeof args.path !== "string") {
    throw new Error("path is required");
  }
  const worktree = stream.worktree_path;
  const absolutePath = isAbsolute(args.path) ? resolve(args.path) : resolve(worktree, args.path);
  const rel = relative(worktree, absolutePath);
  if (rel.startsWith("..") || isAbsolute(rel)) {
    throw new Error(`path is outside the stream worktree: ${args.path}`);
  }
  const languageId = lspLanguageIdForPath(absolutePath);
  if (!languageId) {
    throw new Error(`no LSP configured for file: ${args.path}`);
  }
  const text = readFileSync(absolutePath, "utf8");
  const session = await deps.lspManager.getSession(stream, languageId);
  const uri = fileUri(absolutePath);
  session.syncDocument(uri, text);
  return { stream, absolutePath, relativePath: rel, uri, session, text };
}

function toLspPosition(line: number, column: number): { line: number; character: number } {
  if (!Number.isFinite(line) || !Number.isFinite(column) || line < 1 || column < 1) {
    throw new Error("line and column must be 1-based integers >= 1");
  }
  return { line: Math.trunc(line) - 1, character: Math.trunc(column) - 1 };
}

interface NormalizedLocation {
  path: string;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
}

function normalizeLocation(worktree: string, uri: string, range: unknown): NormalizedLocation | null {
  if (!range || typeof range !== "object") return null;
  const candidate = range as {
    start?: { line?: number; character?: number };
    end?: { line?: number; character?: number };
  };
  const startLine = candidate.start?.line;
  const startChar = candidate.start?.character;
  if (typeof startLine !== "number" || typeof startChar !== "number") return null;
  const endLine = typeof candidate.end?.line === "number" ? candidate.end!.line : startLine;
  const endChar = typeof candidate.end?.character === "number" ? candidate.end!.character : startChar;
  const absolute = uriToPath(uri);
  const rel = relative(worktree, absolute);
  return {
    path: rel || absolute,
    line: startLine + 1,
    column: startChar + 1,
    endLine: endLine + 1,
    endColumn: endChar + 1,
  };
}

function uriToPath(uri: string): string {
  if (!uri.startsWith("file://")) return uri;
  try {
    return decodeURIComponent(new URL(uri).pathname);
  } catch {
    return uri;
  }
}

function normalizeDefinitionResult(worktree: string, result: unknown): NormalizedLocation[] {
  const items = Array.isArray(result) ? result : result ? [result] : [];
  const locations: NormalizedLocation[] = [];
  for (const item of items) {
    if (!item || typeof item !== "object") continue;
    const candidate = item as {
      uri?: string;
      targetUri?: string;
      range?: unknown;
      targetSelectionRange?: unknown;
      targetRange?: unknown;
    };
    const targetUri = candidate.targetUri ?? candidate.uri;
    const range = candidate.targetSelectionRange ?? candidate.targetRange ?? candidate.range;
    if (!targetUri) continue;
    const normalized = normalizeLocation(worktree, targetUri, range);
    if (normalized) locations.push(normalized);
  }
  return locations;
}

function normalizeHoverContents(contents: unknown): string {
  if (!contents) return "";
  if (typeof contents === "string") return contents;
  if (Array.isArray(contents)) {
    return contents.map((entry) => normalizeHoverContents(entry)).filter(Boolean).join("\n\n");
  }
  if (typeof contents === "object") {
    const candidate = contents as { value?: unknown; language?: unknown };
    if (typeof candidate.value === "string") {
      if (typeof candidate.language === "string" && candidate.language) {
        return `\`\`\`${candidate.language}\n${candidate.value}\n\`\`\``;
      }
      return candidate.value;
    }
  }
  return "";
}

function severityLabel(severity?: number): "error" | "warning" | "info" | "hint" {
  switch (severity) {
    case 1: return "error";
    case 2: return "warning";
    case 3: return "info";
    default: return "hint";
  }
}

/**
 * Known-false-positive filter. tsserver (and similar) emits
 * `Cannot find module 'bun:test'` whenever a `.test.ts` file imports
 * from `bun:test` because bun's ambient types aren't in the tsconfig
 * the LSP uses. The diagnostic is wrong — bun resolves it at runtime —
 * and it fires on essentially every test file, drowning out real
 * errors. Filtering here means no agent-facing surface (lsp_diagnostics
 * MCP tool, future publishDiagnostics injections) ever sees it.
 */
export function isKnownFalsePositiveDiagnostic(diagnostic: LspDiagnostic): boolean {
  const msg = diagnostic.message ?? "";
  // ts2307 "Cannot find module" against bun: virtual modules.
  if (/Cannot find module '(bun:[^']+)'/.test(msg)) return true;
  return false;
}

function normalizeDiagnostics(diagnostics: LspDiagnostic[]): Array<{
  severity: string;
  message: string;
  source?: string;
  code?: string | number;
  range: { line: number; column: number; endLine: number; endColumn: number };
}> {
  return diagnostics
    .filter((diagnostic) => !isKnownFalsePositiveDiagnostic(diagnostic))
    .map((diagnostic) => ({
      severity: severityLabel(diagnostic.severity),
      message: diagnostic.message,
      source: diagnostic.source,
      code: diagnostic.code,
      range: {
        line: diagnostic.range.start.line + 1,
        column: diagnostic.range.start.character + 1,
        endLine: diagnostic.range.end.line + 1,
        endColumn: diagnostic.range.end.character + 1,
      },
    }));
}

const POSITION_SCHEMA = {
  streamId: { type: "string", description: "Optional stream id. Defaults to the current stream." },
  path: { type: "string", description: "File path relative to the stream worktree." },
  line: { type: "number", description: "1-based line number." },
  column: { type: "number", description: "1-based column number (character offset + 1)." },
} as const;

export function buildLspMcpTools(deps: LspMcpToolDeps): ToolDef[] {
  return [
    {
      name: "newde__lsp_definition",
      description:
        "Jump to the definition of the symbol at (line, column) in the given file. Returns an array of locations (path relative to the worktree, plus 1-based line/column ranges). Works for any language configured in newde.yaml's lsp.servers.",
      inputSchema: {
        type: "object",
        properties: POSITION_SCHEMA,
        required: ["path", "line", "column"],
      },
      handler: async (args: PositionArgs) => {
        const doc = await resolveAndOpen(deps, args);
        const result = await doc.session.request("textDocument/definition", {
          textDocument: { uri: doc.uri },
          position: toLspPosition(args.line, args.column),
        });
        return { locations: normalizeDefinitionResult(doc.stream.worktree_path, result) };
      },
    },
    {
      name: "newde__lsp_references",
      description:
        "Find all references to the symbol at (line, column). Returns locations with worktree-relative paths and 1-based ranges.",
      inputSchema: {
        type: "object",
        properties: POSITION_SCHEMA,
        required: ["path", "line", "column"],
      },
      handler: async (args: PositionArgs) => {
        const doc = await resolveAndOpen(deps, args);
        const result = await doc.session.request<unknown[]>("textDocument/references", {
          textDocument: { uri: doc.uri },
          position: toLspPosition(args.line, args.column),
          context: { includeDeclaration: true },
        });
        return { locations: normalizeDefinitionResult(doc.stream.worktree_path, result) };
      },
    },
    {
      name: "newde__lsp_hover",
      description:
        "Retrieve hover documentation (type info, JSDoc / docstrings) for the symbol at (line, column). Returns { markdown, range }.",
      inputSchema: {
        type: "object",
        properties: POSITION_SCHEMA,
        required: ["path", "line", "column"],
      },
      handler: async (args: PositionArgs) => {
        const doc = await resolveAndOpen(deps, args);
        const result = await doc.session.request<{ contents?: unknown; range?: unknown } | null>(
          "textDocument/hover",
          {
            textDocument: { uri: doc.uri },
            position: toLspPosition(args.line, args.column),
          },
        );
        if (!result) return { markdown: "", range: null };
        const markdown = normalizeHoverContents(result.contents);
        const range = normalizeLocation(doc.stream.worktree_path, doc.uri, result.range);
        return {
          markdown,
          range: range
            ? { line: range.line, column: range.column, endLine: range.endLine, endColumn: range.endColumn }
            : null,
        };
      },
    },
    {
      name: "newde__lsp_diagnostics",
      description:
        "Return the language server's current diagnostics (errors, warnings, hints) for the given file. Opens the file in the server if needed, waits briefly for the first batch of diagnostics, and returns normalized entries with 1-based ranges.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: POSITION_SCHEMA.streamId,
          path: POSITION_SCHEMA.path,
        },
        required: ["path"],
      },
      handler: async (args: BaseArgs) => {
        const doc = await resolveAndOpen(deps, args);
        const cached = doc.session.getDiagnostics(doc.uri);
        const diagnostics = cached ?? (await doc.session.waitForDiagnostics(doc.uri, DIAGNOSTICS_WAIT_MS));
        return { diagnostics: normalizeDiagnostics(diagnostics) };
      },
    },
  ];
}
