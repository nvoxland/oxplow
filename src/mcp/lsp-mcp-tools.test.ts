import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { LspDiagnostic, LspSession } from "../lsp/lsp.js";
import { fileUri, registerLanguageServer, unregisterLanguageServer } from "../lsp/lsp.js";
import type { Stream } from "../persistence/stream-store.js";
import { buildLspMcpTools, type LspManagerLike } from "./lsp-mcp-tools.js";

interface FakeSession {
  syncDocument: (uri: string, text: string) => void;
  closeDocument: (uri: string) => void;
  request: (method: string, params: unknown) => Promise<unknown>;
  getDiagnostics: (uri: string) => LspDiagnostic[] | undefined;
  waitForDiagnostics: (uri: string, timeoutMs: number) => Promise<LspDiagnostic[]>;
}

function buildFakeManager(overrides: Partial<FakeSession> = {}) {
  const calls: Array<{ method: string; params: unknown }> = [];
  const syncs: Array<{ uri: string; text: string }> = [];
  const diagnosticsByUri = new Map<string, LspDiagnostic[]>();
  const session: FakeSession = {
    syncDocument: (uri, text) => { syncs.push({ uri, text }); },
    closeDocument: () => {},
    request: async (method, params) => {
      calls.push({ method, params });
      return null;
    },
    getDiagnostics: (uri) => diagnosticsByUri.get(uri),
    waitForDiagnostics: async (uri) => diagnosticsByUri.get(uri) ?? [],
    ...overrides,
  };
  const manager: LspManagerLike = {
    getSession: async () => session as unknown as LspSession,
  };
  return { manager, session, calls, syncs, diagnosticsByUri };
}

const fakeStream: Stream = {
  id: "stream-1",
  title: "test",
  worktree_path: "",
} as unknown as Stream;

let tmpDir: string;
let workDir: string;

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), "newde-lsp-tools-"));
  workDir = join(tmpDir, "wt");
  mkdirSync(workDir);
  (fakeStream as any).worktree_path = workDir;
  registerLanguageServer({ languageId: "python", extensions: [".py"], command: "stub", args: [] });
});

afterEach(() => {
  unregisterLanguageServer("python");
  rmSync(tmpDir, { recursive: true, force: true });
});

describe("newde__lsp_definition", () => {
  test("converts 1-based line/column to 0-based LSP position and reads file", async () => {
    writeFileSync(join(workDir, "app.py"), "print('hello')\n");
    const { manager, calls, syncs } = buildFakeManager({
      request: async (method, params) => {
        expect(method).toBe("textDocument/definition");
        expect(params).toEqual({
          textDocument: { uri: fileUri(join(workDir, "app.py")) },
          position: { line: 2, character: 5 },
        });
        return {
          uri: fileUri(join(workDir, "app.py")),
          range: { start: { line: 0, character: 6 }, end: { line: 0, character: 11 } },
        };
      },
    });
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_definition")!;
    const out = await tool.handler({ path: "app.py", line: 3, column: 6 });
    expect(out).toEqual({
      locations: [
        {
          path: "app.py",
          line: 1,
          column: 7,
          endLine: 1,
          endColumn: 12,
        },
      ],
    });
    expect(syncs).toEqual([{ uri: fileUri(join(workDir, "app.py")), text: "print('hello')\n" }]);
  });

  test("throws on unknown language", async () => {
    writeFileSync(join(workDir, "notes.unknown"), "");
    const { manager } = buildFakeManager();
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_definition")!;
    await expect(tool.handler({ path: "notes.unknown", line: 1, column: 1 })).rejects.toThrow(/no LSP/i);
  });

  test("throws when path escapes the worktree", async () => {
    const { manager } = buildFakeManager();
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_definition")!;
    await expect(tool.handler({ path: "../outside.py", line: 1, column: 1 })).rejects.toThrow(/outside/i);
  });
});

describe("newde__lsp_hover", () => {
  test("normalizes markup content", async () => {
    writeFileSync(join(workDir, "a.py"), "x = 1\n");
    const { manager } = buildFakeManager({
      request: async () => ({
        contents: { kind: "markdown", value: "**x**: int" },
        range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } },
      }),
    });
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_hover")!;
    const out = await tool.handler({ path: "a.py", line: 1, column: 1 });
    expect(out).toEqual({
      markdown: "**x**: int",
      range: { line: 1, column: 1, endLine: 1, endColumn: 2 },
    });
  });
});

describe("newde__lsp_diagnostics", () => {
  test("returns diagnostics with 1-based ranges and severity labels", async () => {
    writeFileSync(join(workDir, "a.py"), "x = 1\n");
    const uri = fileUri(join(workDir, "a.py"));
    const diagnosticsByUri = new Map<string, LspDiagnostic[]>();
    diagnosticsByUri.set(uri, [
      {
        range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } },
        severity: 1,
        message: "bad",
        source: "pyright",
      },
    ]);
    const { manager } = buildFakeManager({
      getDiagnostics: (u) => diagnosticsByUri.get(u),
      waitForDiagnostics: async (u) => diagnosticsByUri.get(u) ?? [],
    });
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_diagnostics")!;
    const out = await tool.handler({ path: "a.py" });
    expect(out).toEqual({
      diagnostics: [
        {
          severity: "error",
          message: "bad",
          source: "pyright",
          range: { line: 1, column: 1, endLine: 1, endColumn: 2 },
        },
      ],
    });
  });
});

describe("newde__lsp_references", () => {
  test("returns all references as worktree-relative paths", async () => {
    writeFileSync(join(workDir, "a.py"), "x = 1\n");
    const { manager } = buildFakeManager({
      request: async () => [
        {
          uri: fileUri(join(workDir, "a.py")),
          range: { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } },
        },
        {
          uri: fileUri(join(workDir, "b.py")),
          range: { start: { line: 4, character: 2 }, end: { line: 4, character: 3 } },
        },
      ],
    });
    const tools = buildLspMcpTools({ resolveStream: () => fakeStream, lspManager: manager });
    const tool = tools.find((t) => t.name === "newde__lsp_references")!;
    const out = await tool.handler({ path: "a.py", line: 1, column: 1 });
    expect(out).toEqual({
      locations: [
        { path: "a.py", line: 1, column: 1, endLine: 1, endColumn: 2 },
        { path: "b.py", line: 5, column: 3, endLine: 5, endColumn: 4 },
      ],
    });
  });
});
