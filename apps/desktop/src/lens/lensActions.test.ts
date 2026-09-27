import { expect, test } from "bun:test";

import type { LensAction, LensRun } from "../tauri-bridge/generated/bindings.js";
import { performLensAction, type LensActionDeps } from "./lensActions.js";

const run = { lens: { id: "acme/prs" }, params: { repo: "x" } } as unknown as LensRun;
const action = (kind: LensAction["kind"], over: Partial<LensAction> = {}): LensAction => ({
  id: kind,
  kind,
  label: kind,
  source: null,
  ...over,
});

function deps(result: { text?: string; rows?: Record<string, number> } = {}, fail?: string) {
  const log: string[] = [];
  const d: LensActionDeps = {
    runLensAction: async (id, a, params, streamId) => {
      log.push(`run ${id} ${a} ${JSON.stringify(params)} ${streamId}`);
      if (fail) throw new Error(fail);
      return { text: result.text ?? null, report: result.rows ? { extension: "acme", sourceId: "prs", rowCounts: result.rows } : null };
    },
    copyText: async (t) => void log.push(`copy ${t}`),
    insertIntoAgent: (t) => log.push(`agent ${t}`),
    toast: (m) => log.push(`toast ${m}`),
    recordError: (label, m) => log.push(`error ${label}: ${m}`),
  };
  return { d, log };
}

test("copy copies the backend's text for the run's params", async () => {
  const { d, log } = deps({ text: "| a |" });
  expect(await performLensAction(action("copy"), run, "str1", d)).toBe("done");
  expect(log).toEqual([`run acme/prs copy {"repo":"x"} str1`, "copy | a |"]);
});

test("add-to-context hands the lens and its params to the agent locally", async () => {
  const { d, log } = deps();
  await performLensAction(action("add-to-context"), run, null, d);
  expect(log).toEqual(['agent [oxplow lens acme/prs repo="x"] ']);
});

test("run-source reports what synced, and a refusal lands as an op error", async () => {
  const ok = deps({ rows: { pr: 12 } });
  await performLensAction(action("run-source", { label: "Sync PRs", source: "acme/prs" }), run, null, ok.d);
  expect(ok.log[1]).toBe("toast Synced acme/prs: 12 pr.");
  const bad = deps({}, "needs a person's approval first");
  expect(await performLensAction(action("run-source", { label: "Sync PRs", source: "acme/prs" }), run, null, bad.d)).toBe("failed");
  expect(bad.log[1]).toBe("error Sync PRs: Error: needs a person's approval first");
});
