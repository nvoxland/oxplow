import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { proposalForSetting, proposalsFromResult, summarizeProposal, type Proposal } from "./proposals.js";

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
