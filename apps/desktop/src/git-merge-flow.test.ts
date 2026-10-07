import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { OpOutcome } from "./api.js";

// End-to-end check of a UI-initiated background git op (the branch
// picker's "Merge X into Y"). The merge runs through
// `runAsBackgroundTask`: start a BackgroundTask row, run the git op
// detached, complete the row, and let `awaitGitOp` resolve off the
// `backgroundTasksChanged` event. This test pins that the op actually
// fires and its result propagates — i.e. the merge does NOT silently
// no-op — using string `bg-` ids throughout (the flow never touches
// `get_task`).
//
// The merge is the `oxplow.vcs.merge` bus command (P5.B6), so `runCommand` is
// what the op calls. We override only `listen` on the transport module
// and only the background-task commands and `runCommand` on the bindings
// module, spreading the
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

describe("vcsMerge — UI-initiated background VCS op", () => {
  let handlers: Handler[];
  let bgTasks: Map<string, BgRow>;
  let runs: Array<[string, unknown, boolean]>;
  let mergeOutcome: OpOutcome;
  let api: typeof import("./api.js");

  beforeEach(async () => {
    handlers = [];
    bgTasks = new Map();
    runs = [];
    mergeOutcome = { success: true, log: "Merge made by 'ort'.", conflicts: [], auto_resolved: 0 };
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
        runCommand: async (name: string, input: unknown, confirmed: boolean) => {
          runs.push([name, input, confirmed]);
          if (!confirmed) {
            return {
              status: "error",
              error: { code: "NEEDS_CONFIRMATION", message: `\`${name}\` needs confirmation`, cause: null },
            };
          }
          return {
            status: "ok",
            data: { result: mergeOutcome, audit_id: 1, event_id: null, inverse: null },
          };
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

    // A fresh `api.js` over the mocks above, installed as the one every
    // importer (`git-op.js` too) sees: another test file may have mocked
    // `api.js` already, and that copy is wired to the real transport.
    api = await import("./api.js?merge-flow");
    const fresh = api;
    mock.module("./api.js", () => fresh);
  });

  afterEach(() => {
    mock.restore();
  });

  test("runs vcs.merge for the stream, confirmed, and resolves the kickoff with its outcome", async () => {
    const { awaitGitOp } = await import("./git-op.js");
    const result = await awaitGitOp(await api.vcsMerge("str1", "feature", true));

    // The op actually ran (not silently dropped) with the right input.
    expect(runs).toEqual([["oxplow.vcs.merge", { stream: "str1", rev: "feature" }, true]]);
    // …and its outcome propagated back through the background-task row.
    expect(result.success).toBe(true);
    expect(result.log).toContain("Merge made");

    // The background-task id is a string (`bg-*`), so nothing in this
    // flow ever hands a numeric id to a string-typed command.
    const [id] = [...bgTasks.keys()];
    expect(typeof id).toBe("string");
    expect(id.startsWith("bg-")).toBe(true);
  });

  test("surfaces a conflicted merge instead of swallowing it", async () => {
    mergeOutcome = { success: false, log: "CONFLICT (content)", conflicts: ["a.ts"], auto_resolved: 0 };
    const { awaitGitOp, gitOpErrorMessage } = await import("./git-op.js");
    const result = await awaitGitOp(await api.vcsMerge("str1", "feature", true));

    expect(result.success).toBe(false);
    expect(gitOpErrorMessage(result, "merge failed")).toBe("Conflicts in a.ts");
  });

  test("an unconfirmed merge is refused and the task fails with the reason", async () => {
    const { awaitGitOp, gitOpErrorMessage } = await import("./git-op.js");
    const result = await awaitGitOp(await api.vcsMerge("str1", "feature", false));

    expect(result.success).toBe(false);
    expect(gitOpErrorMessage(result, "merge failed")).toContain("needs confirmation");
  });
});
