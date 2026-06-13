import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

// End-to-end check of a UI-initiated background git op (the branch
// picker's "Merge X into Y"). The merge runs through
// `runAsBackgroundTask`: start a BackgroundTask row, run the git op
// detached, complete the row, and let `awaitGitOp` resolve off the
// `backgroundTasksChanged` event. This test pins that the op actually
// fires and its result propagates — i.e. the merge does NOT silently
// no-op — using string `bg-` ids throughout (the flow never touches
// `get_task`).
//
// We override only `listen` on the transport module and only the five
// background-task/merge commands on the bindings module, spreading the
// real modules so no other export is dropped for sibling test files.

type Handler = (e: { payload: unknown }) => void;

interface BgRow {
  id: string;
  kind: string;
  label: string;
  detail: string | null;
  status: "running" | "done" | "failed";
  progress: number | null;
  started_at: number;
  ended_at: number | null;
  error: string | null;
  result_json: string | null;
}

const realTransport = await import("./tauri-bridge/transport.js");
const realBindings = await import("./tauri-bridge/generated/bindings.js");

describe("gitMergeInto — UI-initiated background git op", () => {
  let handlers: Handler[];
  let bgTasks: Map<string, BgRow>;
  let mergeCalls: Array<[unknown, unknown]>;
  let mergeOutcome: { success: boolean; stdout: string; stderr: string; status: number | null };
  let api: typeof import("./api.js");

  beforeEach(async () => {
    handlers = [];
    bgTasks = new Map();
    mergeCalls = [];
    mergeOutcome = { success: true, stdout: "Merge made by 'ort'.", stderr: "", status: 0 };
    let seq = 0;

    const emit = (payload: unknown) => {
      for (const h of [...handlers]) h({ payload });
    };

    mock.module("./tauri-bridge/transport.js", () => ({
      ...realTransport,
      listen: async (_channel: string, cb: Handler) => {
        handlers.push(cb);
        return () => {
          handlers = handlers.filter((h) => h !== cb);
        };
      },
    }));

    mock.module("./tauri-bridge/generated/bindings.js", () => ({
      ...realBindings,
      commands: {
        ...realBindings.commands,
        startBackgroundTask: async (kind: string, label: string, detail: string | null) => {
          const id = `bg-${++seq}`;
          bgTasks.set(id, {
            id,
            kind,
            label,
            detail,
            status: "running",
            progress: null,
            started_at: 0,
            ended_at: null,
            error: null,
            result_json: null,
          });
          return { status: "ok", data: bgTasks.get(id) };
        },
        gitMergeInto: async (streamId: unknown, source: unknown) => {
          mergeCalls.push([streamId, source]);
          return { status: "ok", data: mergeOutcome };
        },
        completeBackgroundTask: async (id: string, resultJson: string | null) => {
          const t = bgTasks.get(id)!;
          t.status = "done";
          t.ended_at = 1;
          t.result_json = resultJson;
          emit({ kind: "backgroundTasksChanged" });
          return { status: "ok", data: null };
        },
        failBackgroundTask: async (id: string, error: string) => {
          const t = bgTasks.get(id)!;
          t.status = "failed";
          t.ended_at = 1;
          t.error = error;
          emit({ kind: "backgroundTasksChanged" });
          return { status: "ok", data: null };
        },
        getBackgroundTask: async (id: string) => ({ status: "ok", data: bgTasks.get(id) ?? null }),
      },
    }));

    api = await import("./api.js");
  });

  afterEach(() => {
    mock.restore();
  });

  test("invokes git_merge_into with the stream id and resolves the kickoff with the op result", async () => {
    const { awaitGitOp } = await import("./git-op.js");
    const result = await awaitGitOp(await api.gitMergeInto("str1", "feature"));

    // The git op actually ran (not silently dropped) with the right args.
    expect(mergeCalls).toEqual([["str1", "feature"]]);
    // …and its result propagated back through the background-task row.
    expect(result.success).toBe(true);
    expect(result.stdout).toContain("Merge made");

    // The background-task id is a string (`bg-*`), so nothing in this
    // flow ever hands a numeric id to a string-typed command.
    const [id] = [...bgTasks.keys()];
    expect(typeof id).toBe("string");
    expect(id.startsWith("bg-")).toBe(true);
  });

  test("surfaces a failed merge instead of swallowing it", async () => {
    mergeOutcome = { success: false, stdout: "", stderr: "CONFLICT (content)", status: 1 };
    const { awaitGitOp, gitOpErrorMessage } = await import("./git-op.js");
    const result = await awaitGitOp(await api.gitMergeInto("str1", "feature"));

    expect(mergeCalls).toEqual([["str1", "feature"]]);
    expect(result.success).toBe(false);
    expect(gitOpErrorMessage(result, "merge failed")).toContain("CONFLICT");
  });
});
