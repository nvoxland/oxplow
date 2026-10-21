import { describe, expect, test } from "bun:test";
import {
  _setApplyEditResponderForTests,
  handleLspSessionEvent,
  LspClient,
  registerLspApplyEditHandler,
  relativePathFromFileUri,
  streamFileUri,
  toEditorNavigationTarget,
  type LspDiagnostic,
} from "./lsp.js";

const stream = {
  id: "s-1",
  title: "proj1",
  summary: "",
  branch: "main",
  branch_ref: "refs/heads/main",
  branch_source: "local" as const,
  worktree_path: "/tmp/proj1",
  created_at: "2024-01-01T00:00:00.000Z",
  updated_at: "2024-01-01T00:00:00.000Z",
  panes: { working: "a:1.0", talking: "a:1.1" },
  resume: { working_session_id: "", talking_session_id: "" },
};

describe("streamFileUri helpers", () => {
  test("builds a real file URI inside the stream worktree", () => {
    expect(streamFileUri(stream, "src/App.tsx")).toBe("file:///tmp/proj1/src/App.tsx");
  });

  test("maps file URIs back to stream-relative paths", () => {
    expect(relativePathFromFileUri(stream, "file:///tmp/proj1/src/App.tsx")).toBe("src/App.tsx");
    expect(relativePathFromFileUri(stream, "file:///tmp/other/App.tsx")).toBeNull();
  });

  test("creates editor navigation targets from LSP locations", () => {
    expect(
      toEditorNavigationTarget(stream, "file:///tmp/proj1/src/App.tsx", {
        start: { line: 4, character: 2 },
      }),
    ).toEqual({
      path: "src/App.tsx",
      line: 5,
      column: 3,
    });
  });
});

describe("LspClient session-event demux", () => {
  const diag: LspDiagnostic = {
    range: { start: { line: 0, character: 0 }, end: { line: 0, character: 3 } },
    severity: 1,
    message: "boom",
  };

  test("publishDiagnostics reaches only the matching (stream, language) client", () => {
    const tsClient = new LspClient("s-1", "typescript");
    const rustClient = new LspClient("s-1", "rust");
    const otherStream = new LspClient("s-2", "typescript");
    const got: { client: string; uri: string; count: number }[] = [];
    tsClient.onDiagnostics((uri, d) => got.push({ client: "ts", uri, count: d.length }));
    rustClient.onDiagnostics((uri, d) => got.push({ client: "rust", uri, count: d.length }));
    otherStream.onDiagnostics((uri, d) => got.push({ client: "s2", uri, count: d.length }));
    try {
      handleLspSessionEvent({
        kind: "serverNotification",
        streamId: "s-1",
        language: "typescript",
        method: "textDocument/publishDiagnostics",
        params: { uri: "file:///tmp/proj1/a.ts", diagnostics: [diag] },
      });
      expect(got).toEqual([{ client: "ts", uri: "file:///tmp/proj1/a.ts", count: 1 }]);
    } finally {
      tsClient.dispose();
      rustClient.dispose();
      otherStream.dispose();
    }
  });

  test("sessionStatus crashed surfaces a status message; ready clears it", () => {
    const client = new LspClient("s-1", "typescript");
    const statuses: (string | null)[] = [];
    client.onStatus((m) => statuses.push(m));
    try {
      handleLspSessionEvent({
        kind: "sessionStatus",
        streamId: "s-1",
        language: "typescript",
        status: "crashed",
        message: "language server exited",
      });
      handleLspSessionEvent({
        kind: "sessionStatus",
        streamId: "s-1",
        language: "typescript",
        status: "ready",
        message: null,
      });
      expect(statuses).toHaveLength(2);
      expect(statuses[0]).toMatch(/crashed/i);
      expect(statuses[1]).toBeNull();
    } finally {
      client.dispose();
    }
  });

  test("applyEditRequest routes to the registered handler and answers with its verdict", async () => {
    const responses: { token: number; applied: boolean; reason?: string }[] = [];
    _setApplyEditResponderForTests(async (token, applied, reason) => {
      responses.push({ token, applied, reason });
    });
    const unregister = registerLspApplyEditHandler(async (req) => {
      if (req.streamId !== "s-1") return null;
      expect(req.edit).toEqual({ changes: {} });
      return { applied: true };
    });
    try {
      handleLspSessionEvent({
        kind: "applyEditRequest",
        streamId: "s-1",
        language: "typescript",
        token: 7,
        label: "refactor",
        edit: { changes: {} },
      });
      await Bun.sleep(0);
      expect(responses).toEqual([{ token: 7, applied: true, reason: undefined }]);
    } finally {
      unregister();
      _setApplyEditResponderForTests(null);
    }
  });

  test("applyEditRequest with no willing handler answers applied:false", async () => {
    const responses: { token: number; applied: boolean; reason?: string }[] = [];
    _setApplyEditResponderForTests(async (token, applied, reason) => {
      responses.push({ token, applied, reason });
    });
    const unregister = registerLspApplyEditHandler(async (req) =>
      req.streamId === "s-other" ? { applied: true } : null,
    );
    try {
      handleLspSessionEvent({
        kind: "applyEditRequest",
        streamId: "s-1",
        language: "typescript",
        token: 8,
        label: null,
        edit: {},
      });
      await Bun.sleep(0);
      expect(responses).toHaveLength(1);
      expect(responses[0].applied).toBe(false);
      expect(responses[0].reason).toMatch(/no editor/i);
    } finally {
      unregister();
      _setApplyEditResponderForTests(null);
    }
  });

  test("disposed clients receive nothing", () => {
    const client = new LspClient("s-1", "typescript");
    const statuses: (string | null)[] = [];
    client.onStatus((m) => statuses.push(m));
    client.dispose();
    handleLspSessionEvent({
      kind: "sessionStatus",
      streamId: "s-1",
      language: "typescript",
      status: "crashed",
      message: null,
    });
    expect(statuses).toHaveLength(0);
  });
});
