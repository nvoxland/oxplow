import { describe, expect, test } from "bun:test";

import {
  applyNormalizedWorkspaceEdit,
  partitionByOpenModel,
  type WorkspaceEditIO,
} from "./lsp-workspace-edit.js";

class FakeRange {
  constructor(
    public startLineNumber: number,
    public startColumn: number,
    public endLineNumber: number,
    public endColumn: number,
  ) {}
}

const fakeMonaco = { Range: FakeRange };

function fakeModel(initial: string) {
  const ops: { range: FakeRange; text: string }[] = [];
  return {
    content: initial,
    ops,
    pushEditOperations(_sel: unknown, edits: { range: FakeRange; text: string }[]) {
      ops.push(...edits);
    },
  };
}

const range = (sl: number, sc: number, el: number, ec: number) => ({
  start: { line: sl, character: sc },
  end: { line: el, character: ec },
});

function makeIO(opts: {
  models?: Record<string, ReturnType<typeof fakeModel>>;
  files?: Record<string, string>;
}): WorkspaceEditIO & { written: Record<string, string> } {
  const written: Record<string, string> = {};
  return {
    written,
    findModel: (uri) => opts.models?.[uri] ?? null,
    pathFromUri: (uri) =>
      uri.startsWith("file:///wt/") ? uri.slice("file:///wt/".length) : null,
    readFile: async (path) => {
      const content = opts.files?.[path];
      if (content === undefined) throw new Error(`no such file: ${path}`);
      return content;
    },
    writeFile: async (path, content) => {
      written[path] = content;
    },
  };
}

describe("partitionByOpenModel", () => {
  test("splits files by model presence", () => {
    const models = { "file:///wt/open.ts": fakeModel("x") };
    const io = makeIO({ models });
    const normalized = {
      files: [
        { uri: "file:///wt/open.ts", edits: [] },
        { uri: "file:///wt/closed.ts", edits: [] },
      ],
      skippedFileOps: 0,
    };
    const { open, closed } = partitionByOpenModel(io, normalized.files);
    expect(open.map((f) => f.uri)).toEqual(["file:///wt/open.ts"]);
    expect(closed.map((f) => f.uri)).toEqual(["file:///wt/closed.ts"]);
  });
});

describe("applyNormalizedWorkspaceEdit", () => {
  test("open files go through the model, closed files through read-modify-write", async () => {
    const model = fakeModel("let a = 1;\n");
    const io = makeIO({
      models: { "file:///wt/open.ts": model },
      files: { "src/closed.ts": "use(a);\n" },
    });
    const result = await applyNormalizedWorkspaceEdit(fakeMonaco, io, {
      files: [
        { uri: "file:///wt/open.ts", edits: [{ range: range(0, 4, 0, 5), newText: "b" }] },
        {
          uri: "file:///wt/src/closed.ts",
          edits: [{ range: range(0, 4, 0, 5), newText: "b" }],
        },
      ],
      skippedFileOps: 0,
    });
    expect(result.appliedFiles).toBe(2);
    expect(result.failures).toEqual([]);
    expect(model.ops).toHaveLength(1);
    expect(model.ops[0].text).toBe("b");
    expect(model.ops[0].range).toMatchObject({ startLineNumber: 1, startColumn: 5 });
    expect(io.written["src/closed.ts"]).toBe("use(b);\n");
  });

  test("failures are collected per file and don't abort the rest", async () => {
    const io = makeIO({ files: { "ok.ts": "a\n" } });
    const result = await applyNormalizedWorkspaceEdit(fakeMonaco, io, {
      files: [
        { uri: "file:///wt/missing.ts", edits: [{ range: range(0, 0, 0, 1), newText: "x" }] },
        { uri: "file:///elsewhere/out.ts", edits: [{ range: range(0, 0, 0, 1), newText: "x" }] },
        { uri: "file:///wt/ok.ts", edits: [{ range: range(0, 0, 0, 1), newText: "x" }] },
      ],
      skippedFileOps: 0,
    });
    expect(result.appliedFiles).toBe(1);
    expect(result.failures).toHaveLength(2);
    expect(result.failures[0]).toContain("missing.ts");
    expect(result.failures[1]).toContain("outside");
    expect(io.written["ok.ts"]).toBe("x\n");
  });
});
