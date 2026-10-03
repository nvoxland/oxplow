import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { proposalForSetting, proposalOfTool, proposalsFromResult, summarizeProposal, type Proposal } from "./proposals.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: ["id", "ref", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "key", "preview", "dry_run"],
    rows,
    truncated: false,
    reads: { models: ["v_command_proposal"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

const proposal = (over: Partial<Proposal>): Proposal => ({
  id: 1,
  ref: "proposal:1",
  createdAt: "t",
  command: "config.set",
  input: {},
  actorKind: "agent",
  actorId: "thr3",
  threadId: 3,
  key: "config:x",
  preview: { command: "config.set", summary: "Set one key.", input: {}, destructive: false },
  dryRun: null,
  decision: "pending",
  decidedAt: null,
  auditId: null,
  ...over,
});

test("rows read as proposals, JSON columns parsed", () => {
  const [p] = proposalsFromResult(
    result([
      [
        4,
        "proposal:4",
        "2026-10-01T00:00:00Z",
        "config.set",
        '{"key":"agentPromptAppend","value":"be brief"}',
        "agent",
        "thr3",
        3,
        "config:agentPromptAppend",
        '{"command":"config.set","summary":"Set one key.","input":{},"destructive":false}',
        '{"key":"agentPromptAppend","before":null,"after":"be brief","changed":true}',
      ],
    ]),
  );
  expect(p).toEqual({
    id: 4,
    ref: "proposal:4",
    createdAt: "2026-10-01T00:00:00Z",
    command: "config.set",
    input: { key: "agentPromptAppend", value: "be brief" },
    actorKind: "agent",
    actorId: "thr3",
    threadId: 3,
    key: "config:agentPromptAppend",
    preview: { command: "config.set", summary: "Set one key.", input: {}, destructive: false },
    dryRun: { key: "agentPromptAppend", before: null, after: "be brief", changed: true },
    decision: "pending",
    decidedAt: null,
    auditId: null,
  });
});

test("a config change summarizes as the key with its before and after", () => {
  const s = summarizeProposal(
    proposal({
      input: { key: "agentPromptAppend", value: "be brief" },
      dryRun: { key: "agentPromptAppend", before: null, after: "be brief", changed: true },
    }),
  );
  expect(s.title).toBe("Set agentPromptAppend");
  expect(s.who).toBe("The agent in thr3");
  expect(s.change).toEqual({ before: "(not set)", after: "be brief" });
  expect(s.children).toEqual([]);
  expect(s.destructive).toBe(false);
  expect(summarizeProposal(proposal({ command: "config.unset", input: { key: "zones" } })).title).toBe("Unset zones");
});

test("a composite lists its children; anything else uses its summary; destructive carries over", () => {
  const s = summarizeProposal(
    proposal({
      command: "review.finish",
      key: "review.finish {}",
      preview: { command: "review.finish", summary: "Finish the review.", input: {}, destructive: true },
      dryRun: { result: null, children: [{ name: "work_item.transition" }, { name: "work_item.delete" }] },
    }),
  );
  expect(s.title).toBe("Finish the review.");
  expect(s.change).toBeNull();
  expect(s.children).toEqual(["work_item.transition", "work_item.delete"]);
  expect(s.destructive).toBe(true);
  expect(summarizeProposal(proposal({ actorKind: "lens", actorId: "acme/x" })).who).toBe("A lens (acme/x) for the agent");
});

test("a setting's pending proposal is the one keyed on it", () => {
  const list = [proposal({ id: 1, key: "config:zones" }), proposal({ id: 2, key: "config:ai" })];
  expect(proposalForSetting(list, "ai")?.id).toBe(2);
  expect(proposalForSetting(list, "lsp")).toBeUndefined();
});


// P9.A3: a thread's proposals carry their decision, and a transcript finds
// the proposal a tool call left.
test("a row's decision reads through; a read that doesn't select it is pending", () => {
  const withDecision = {
    ...result([]),
    columns: ["id", "ref", "command", "preview", "decision", "decided_at"],
    rows: [[4, "proposal:4", "work_item.delete", "{}", "approved", "2026-10-03T00:00:00Z"]],
  } as unknown as SqlQueryResult;
  expect(proposalsFromResult(withDecision)[0]).toMatchObject({ id: 4, decision: "approved", decidedAt: "2026-10-03T00:00:00Z" });
  const pendingOnly = { ...result([]), rows: [[5, "proposal:5", "t", "config.set", "{}", "agent", null, null, "k", "{}", null]] } as unknown as SqlQueryResult;
  expect(proposalsFromResult(pendingOnly)[0]).toMatchObject({ id: 5, decision: "pending", decidedAt: null });
});

test("a tool call's proposal is found in its result, from oxplow's command tools only", () => {
  const call = (name: string, text: string[], rawOutput: unknown = null, status = "completed") =>
    ({ id: "t", name, title: name, kind: "other", status, locations: [], rawInput: null, rawOutput, diffs: [], text }) as never;
  const proposed = JSON.stringify({ kind: "proposed", proposal: "proposal:12", message: "waits" });
  expect(proposalOfTool(call("mcp__oxplow__run_command", [proposed]))).toBe("proposal:12");
  // Wherever the agent's client puts the result: escaped inside a string, or raw output.
  expect(proposalOfTool(call("mcp__oxplow__run_command", [JSON.stringify(proposed)]))).toBe("proposal:12");
  expect(proposalOfTool(call("mcp__oxplow__run_command", [], { kind: "proposed", proposal: "proposal:3" }))).toBe("proposal:3");
  // A lens action's proposal comes back as the refusal's message.
  expect(
    proposalOfTool(call("mcp__oxplow__run_lens_action", ["`work_item.delete` needs a person's approval; it is recorded as proposal:9 and waits"], null, "failed")),
  ).toBe("proposal:9");
  // A run that ran, and other tools quoting such text, name none.
  expect(proposalOfTool(call("mcp__oxplow__run_command", [JSON.stringify({ result: { ok: true } })]))).toBeNull();
  expect(proposalOfTool(call("Bash", [proposed]))).toBeNull();
  expect(proposalOfTool(call("mcp__oxplow__query_sql", [proposed]))).toBeNull();
});

