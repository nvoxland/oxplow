import { expect, test } from "bun:test";

import type { DataEntity } from "../tauri-bridge/generated/bindings.js";
import { entityRows, entitySummary, programRow } from "./dataSectionModel.js";

const entity = (name: string, owner: string, kind: string): DataEntity => ({
  name,
  owner,
  kind,
  description: `${name} rows`,
});

test("entityRows puts core first, formats counts and flags unsynced entities", () => {
  const rows = entityRows(
    [
      entity("v_github_pr", "github", "declared"),
      entity("v_work_item", "core", "sql"),
      entity("v_commit", "core", "sql"),
      entity("v_issues_issue", "issues", "entity"),
    ],
    { v_work_item: { rows: 12345 }, v_commit: { rows: 0 }, v_issues_issue: { rows: 7 } },
  );
  expect(rows.map((r) => r.name)).toEqual(["v_commit", "v_work_item", "v_github_pr", "v_issues_issue"]);
  expect(rows[1]!.rows).toBe(new Intl.NumberFormat().format(12345));
  expect(rows[0]!.rows).toBe("0");
  expect(rows[2]!.rows).toBe("Not synced yet");
  expect(rows[2]!.available).toBe(false);
  expect(rows[3]!.rows).toBe("7");
  expect(entitySummary(rows)).toBe("4 entities · 2 from extensions");
  expect(entitySummary(rows.slice(0, 1))).toBe("1 entity");
});

// tsk1065: counts load one model at a time after the list shows, so a
// model too big to count in time costs its own cell, not the list.
test("a count still loading says so, and one that failed reads as a dash with why", () => {
  const [counting, failed] = entityRows([entity("v_a", "core", "sql"), entity("v_b", "core", "sql")], {
    v_b: { error: "query_sql: timed out after 5s" },
  });
  expect([counting!.rows, counting!.rowsTitle]).toEqual(["Counting…", undefined]);
  expect(failed!.rows).toBe("—");
  expect(failed!.rowsTitle).toBe("Couldn't count it: query_sql: timed out after 5s");
});

test("programRow says what runs and whether it will", () => {
  const m = programRow({ kind: "collector", name: "tests.parse", program: "tools/parse.sh", args: ["--x"], env: [], approved: false });
  expect(m.label).toBe("Collector tests.parse");
  expect(m.command).toBe("tools/parse.sh --x");
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("tools/parse.sh --x");
  expect(programRow({ kind: "collector", name: "repo.n", program: "t.sh", args: [], env: [], approved: true }).status).toBe(
    "Approved on this machine",
  );
  // An ACP agent shows the env it runs with, since that's part of what's approved.
  const agent = programRow({ kind: "acp-agent", name: "mine", program: "tools/agent", args: ["--acp"], env: ["MODE=fast"], approved: false });
  expect([agent.label, agent.command]).toEqual(["ACP agent mine", "MODE=fast tools/agent --acp"]);
});

test("programRow shows a shared extension's advisories as what they'd say", () => {
  const m = programRow({
    kind: "advisories",
    name: "team",
    program: "oxplow/extensions/team/extension.yaml",
    args: ["nag (on prompt, once per effort): SELECT 'x' AS message"],
    env: [],
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Advisories from team");
  expect(m.command).toBe("nag (on prompt, once per effort): SELECT 'x' AS message");
  expect(m.status).toBe("Not approved: they won't reach your agent");
  expect(m.approveTitle).toContain("agent's context");
});

test("programRow shows a provider with the secrets and hosts it gets", () => {
  const m = programRow({
    kind: "provider",
    name: "tracker/issues",
    program: "oxplow/extensions/tracker/bin/provider",
    args: ["--stdio"],
    env: ["ISSUES_URL"],
    credentials: ["token"],
    network: ["api.tracker.example"],
    scopes: [],
    tree: "oxplow/extensions/tracker",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("External provider tracker/issues");
  expect(m.command).toBe(
    "oxplow/extensions/tracker/bin/provider --stdio\nenv: ISSUES_URL\ncredentials: token\nreaches: api.tracker.example",
  );
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("every file in oxplow/extensions/tracker");
});

test("programRow shows an effect as reacting to events from approval on (P8.D9)", () => {
  const m = programRow({
    kind: "effect",
    name: "acme/announce-done",
    program: "oxplow/extensions/acme/effects/announce.star",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    tree: "oxplow/extensions/acme",
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Effect acme/announce-done");
  expect(m.command).toBe("oxplow/extensions/acme/effects/announce.star");
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("events logged after you approve");
  expect(m.approveTitle).toContain("every file in oxplow/extensions/acme");
  expect(m.bundled).toBe(false);
});

// tsk960: a component that declares commands runs them with the viewer's
// rights: the row names its bundle and those commands, and says what
// stays off until it's approved.
test("programRow shows a component with the commands it may run", () => {
  const m = programRow({
    kind: "component",
    name: "github/pr-lifetimes",
    program: "oxplow/extensions/github/components/pr-lifetimes",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    commands: ["oxplow.collector.sync"],
    tree: "oxplow/extensions/github",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Component github/pr-lifetimes");
  expect(m.command).toBe("oxplow/extensions/github/components/pr-lifetimes\nmay run: oxplow.collector.sync");
  expect(m.status).toBe("Not approved: it shows and reads, but can't act");
  expect(m.approveTitle).toContain("with your rights");
  expect(m.approveTitle).toContain("oxplow.collector.sync");
  expect(m.approveTitle).toContain("every file of its bundle");
});

// tsk953: a bundled extension's effect is approved like any other; its
// files come with oxplow, so a new oxplow that changes them asks again.
test("programRow says a bundled effect asks again when a new oxplow changes it", () => {
  const m = programRow({
    kind: "effect",
    name: "oxplow-bundled/verify-unchecked",
    program: "bundled:oxplow-bundled/effects/verify.star",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    tree: "bundled:oxplow-bundled",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.bundled).toBe(true);
  expect(m.command).toBe("effects/verify.star, part of oxplow-bundled (comes with oxplow)");
  expect(m.approveTitle).toContain("comes with oxplow");
  expect(m.approveTitle).toContain("A new oxplow that changes it asks again");
});

import { backfillAsk, backfillDone, backfillRunLabel, canApprove, providerImpactLines } from "./dataSectionModel.js";
import type { ProviderImpact } from "../tauri-bridge/generated/bindings.js";

// A provider's Approve waits for what approving would change; the
// change reads as lines — everything new at first, then what differs.
test("a provider's approve waits for its declaration diff", () => {
  const provider = { kind: "provider", name: "tracker/fake", program: "p", args: [], env: [], approved: false } as never;
  expect(canApprove(provider, undefined)).toBe(false);
  expect(canApprove(provider, "loading")).toBe(false);
  expect(canApprove(provider, { change: "added" } as ProviderImpact)).toBe(true);
  expect(canApprove(provider, { error: "no provider.json" })).toBe(false);
  const collector = { kind: "collector", name: "g", program: "p", args: [], env: [], approved: false } as never;
  expect(canApprove(collector, undefined)).toBe(true);
});

// The wording is the server's (`extension_impact::approval_lines`,
// tested in Rust); the approval row shows the lines it sends.
test("a provider's declaration diff shows the server's lines", () => {
  const impact = {
    id: "fake",
    change: "changed",
    lines: ["Now reaches api.example.com (was none)", "Command `archive` added (destructive)"],
  } as unknown as ProviderImpact;
  expect(providerImpactLines(impact)).toEqual([
    "Now reaches api.example.com (was none)",
    "Command `archive` added (destructive)",
  ]);
});

// P9.B4: a provider whose MCP server is reached by url runs nothing of
// its own here; the row says what the approval does and doesn't cover.
test("programRow says a server by url isn't code this approval covers", () => {
  const row = programRow({
    kind: "provider",
    name: "notes/notes",
    program: "https://mcp.example.com/mcp",
    args: ["mcp/notes.star", "mcp/tools.json", "--auth-env", "NOTES_TOKEN"],
    env: [],
    credentials: ["NOTES_TOKEN"],
    network: ["mcp.example.com"],
    scopes: [],
    tree: "oxplow/extensions/notes",
    remote: true,
    approved: false,
    version: "abc",
  });
  expect(row.command).toBe(
    "MCP server at https://mcp.example.com/mcp\nwith: mcp/notes.star mcp/tools.json --auth-env NOTES_TOKEN\ncredentials: NOTES_TOKEN\nreaches: mcp.example.com",
  );
  expect(row.approveTitle).toContain("The server runs elsewhere: its code isn't part of this approval");
  expect(row.approveTitle).toContain("its tools stop matching the pinned ones");
});

// P9.D5: what a backfill would do, said before it runs, and what it did.
test("a backfill says how many events it would react to, and how it went", () => {
  expect(backfillAsk("acme/mark-done", 0)).toBe("acme/mark-done has reacted to every matching event: nothing to backfill.");
  expect(backfillAsk("acme/mark-done", 1)).toBe(
    "acme/mark-done never reacted to 1 matching event. Backfilling runs it on that event, as it is now; it may call outside oxplow.",
  );
  expect(backfillAsk("acme/mark-done", 3)).toBe(
    "acme/mark-done never reacted to 3 matching events. Backfilling runs it on each, oldest first, as it is now; it may call outside oxplow for every one.",
  );
  expect(backfillDone({ planned: 3, ran: 3, skipped: 0, proposed: 0, failed: 0, remaining: 0 })).toBe("Reacted to 3 of 3 events.");
  expect(backfillDone({ planned: 5, ran: 1, skipped: 1, proposed: 0, failed: 3, remaining: 0, stopped: "the effect was disabled: 3 failures in a row" })).toBe(
    "Reacted to 1 of 5 events; 3 failed, 1 skipped. Stopped: the effect was disabled: 3 failures in a row.",
  );
  expect(backfillDone({ planned: 300, ran: 200, skipped: 0, proposed: 0, failed: 0, remaining: 100 })).toBe(
    "Reacted to 200 of 300 events; 100 remain — run it again for the rest.",
  );
});

// tsk849: a run makes at most a batch of reactions; its button says so.
test("a backfill's button says how many one run reacts to", () => {
  expect(backfillRunLabel(1, 200)).toBe("Run on 1 event");
  expect(backfillRunLabel(200, 200)).toBe("Run on 200 events");
  expect(backfillRunLabel(500, 200)).toBe("Run on the first 200 of 500");
});

