import { expect, test } from "bun:test";

import type { SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { proposalsFromResult, summarizeProposal, type Proposal } from "./proposals.js";

const result = (rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({
    columns: ["id", "uri", "created_at", "command", "input", "actor_kind", "actor_id", "thread_id", "thread_title", "key", "preview", "dry_run"],
    rows,
    truncated: false,
    reads: { datasets: ["v_command_proposal"], tables: [], measures: [] },
    freshness: {},
  }) as unknown as SqlQueryResult;

const proposal = (over: Partial<Proposal>): Proposal => ({
  id: 1,
  uri: "oxplow:oxplow/proposal?id=1",
  createdAt: "t",
  command: "oxplow.config.set",
  input: {},
  actorKind: "agent",
  actorId: "thr3",
  threadId: 3,
  threadTitle: "Fix the cart",
  key: "oxplow:oxplow/config?key=x",
  preview: { command: "oxplow.config.set", summary: "Set one key.", input: {}, destructive: false },
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
        "oxplow:oxplow/proposal?id=4",
        "2026-10-01T00:00:00Z",
        "oxplow.config.set",
        '{"key":"agentPromptAppend","value":"be brief"}',
        "agent",
        "thr3",
        3,
        "Thread",
        "oxplow:oxplow/config?key=agentPromptAppend",
        '{"command":"oxplow.config.set","summary":"Set one key.","input":{},"destructive":false}',
        '{"key":"agentPromptAppend","before":null,"after":"be brief","changed":true}',
      ],
    ]),
  );
  expect(p).toEqual({
    id: 4,
    uri: "oxplow:oxplow/proposal?id=4",
    createdAt: "2026-10-01T00:00:00Z",
    command: "oxplow.config.set",
    input: { key: "agentPromptAppend", value: "be brief" },
    actorKind: "agent",
    actorId: "thr3",
    threadId: 3,
    threadTitle: "Thread",
    key: "oxplow:oxplow/config?key=agentPromptAppend",
    preview: { command: "oxplow.config.set", summary: "Set one key.", input: {}, destructive: false },
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
  expect(s.change).toEqual({ before: "(not set)", after: "be brief" });
  expect(s.children).toEqual([]);
  expect(s.destructive).toBe(false);
  expect(summarizeProposal(proposal({ command: "oxplow.config.unset", input: { key: "zones" } })).title).toBe("Unset zones");
});

test("a composite lists its children; anything else uses its summary; destructive carries over", () => {
  const s = summarizeProposal(
    proposal({
      command: "review.finish",
      key: "review.finish {}",
      preview: { command: "review.finish", summary: "Finish the review.", input: {}, destructive: true },
      dryRun: { result: null, children: [{ name: "oxplow.work_item.transition" }, { name: "oxplow.work_item.delete" }] },
    }),
  );
  expect(s.title).toBe("Finish the review.");
  expect(s.change).toBeNull();
  expect(s.children).toEqual(["oxplow.work_item.transition", "oxplow.work_item.delete"]);
  expect(s.destructive).toBe(true);
});

test("a deploy's proposal lists what it adds", () => {
  const s = summarizeProposal(
    proposal({
      command: "oxplow.extension.deploy",
      preview: {
        command: "oxplow.extension.deploy",
        summary: "`acme` asks for more than it runs with now",
        input: { name: "acme" },
        destructive: false,
        details: ["process — runs programs with your authority; oxplow does not confine them"],
      },
    }),
  );
  expect(s.details).toEqual(["process — runs programs with your authority; oxplow does not confine them"]);
  expect(summarizeProposal(proposal({})).details).toEqual([]);
});

// P9.A3: a thread's proposals carry their decision.
test("a row's decision reads through; a read that doesn't select it is pending", () => {
  const withDecision = {
    ...result([]),
    columns: ["id", "uri", "command", "preview", "decision", "decided_at"],
    rows: [[4, "oxplow:oxplow/proposal?id=4", "oxplow.work_item.delete", "{}", "approved", "2026-10-03T00:00:00Z"]],
  } as unknown as SqlQueryResult;
  expect(proposalsFromResult(withDecision)[0]).toMatchObject({ id: 4, decision: "approved", decidedAt: "2026-10-03T00:00:00Z" });
  const pendingOnly = { ...result([]), rows: [[5, "oxplow:oxplow/proposal?id=5", "t", "oxplow.config.set", "{}", "agent", null, null, "k", "{}", null]] } as unknown as SqlQueryResult;
  expect(proposalsFromResult(pendingOnly)[0]).toMatchObject({ id: 5, decision: "pending", decidedAt: null });
});
