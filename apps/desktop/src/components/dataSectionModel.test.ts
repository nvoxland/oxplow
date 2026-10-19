import { expect, test } from "bun:test";

import type { DataEntity } from "../tauri-bridge/generated/bindings.js";
import { collectorRevoke, entityRows, entitySummary, programRow } from "./dataSectionModel.js";
import type { CollectorListing } from "../api.js";

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
  const m = programRow({ kind: "ai-provider", name: "acme/gemini", program: "ext/agent.star", args: [], env: [], network: [], approved: false } as never);
  expect(m.label).toBe("AI provider acme/gemini");
  expect(m.command).toContain("ext/agent.star");
  expect(m.status).toBe("Not approved: calls through it fail");
  expect(m.approveTitle).toContain("ext/agent.star");
  expect(
    programRow({ kind: "ai-provider", name: "acme/n", program: "t.star", args: [], env: [], network: [], approved: true } as never).status,
  ).toBe("Approved on this machine");
});

test("programRow shows a shared extension's advisories as what they'd say", () => {
  const m = programRow({
    kind: "advisories",
    name: "team",
    program: ".oxplow/extensions/team/extension.yaml",
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
    program: ".oxplow/extensions/tracker/bin/provider",
    args: ["--stdio"],
    env: ["ISSUES_URL"],
    credentials: ["token"],
    network: ["api.tracker.example"],
    scopes: [],
    tree: ".oxplow/extensions/tracker",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("External provider tracker/issues");
  expect(m.command).toBe(
    ".oxplow/extensions/tracker/bin/provider --stdio\nenv: ISSUES_URL\ncredentials: token\nreaches: api.tracker.example",
  );
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("every file in .oxplow/extensions/tracker");
});

// A harness provider launches the person's agent sessions: the command
// its launch answers runs in their terminal, outside its own grants.
test("programRow says an agent harness provider launches agent sessions", () => {
  const m = programRow({
    kind: "provider",
    capability: "agent_harness",
    name: "conductor/fake",
    program: ".oxplow/extensions/conductor/bin/provider",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    tree: ".oxplow/extensions/conductor",
    remote: false,
    approved: false,
    version: "abc",
  } as never);
  expect(m.label).toBe("Agent harness conductor/fake");
  expect(m.command).toBe(
    ".oxplow/extensions/conductor/bin/provider\nlaunches agent sessions: the command it answers runs in your terminal",
  );
  expect(m.approveTitle).toContain("runs in your terminal");
  expect(m.approveTitle).toContain("session's bearer");
});

test("programRow shows an effect as reacting to events from approval on (P8.D9)", () => {
  const m = programRow({
    kind: "effect",
    name: "acme/announce-done",
    program: ".oxplow/extensions/acme/effects/announce.star",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    tree: ".oxplow/extensions/acme",
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Effect acme/announce-done");
  expect(m.command).toBe(".oxplow/extensions/acme/effects/announce.star");
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("events logged after you approve");
  expect(m.approveTitle).toContain("every file in .oxplow/extensions/acme");
  expect(m.showsSource).toBe(false);
});

// tsk960: a component that declares commands runs them with the viewer's
// rights: the row names its bundle and those commands, and says what
// stays off until it's approved.
test("programRow shows a component with the commands it may run", () => {
  const m = programRow({
    kind: "component",
    name: "github/pr-lifetimes",
    program: ".oxplow/extensions/github/components/pr-lifetimes",
    args: [],
    env: [],
    credentials: [],
    network: [],
    scopes: [],
    commands: ["oxplow.collector.sync"],
    tree: ".oxplow/extensions/github",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Component github/pr-lifetimes");
  expect(m.command).toBe(".oxplow/extensions/github/components/pr-lifetimes\nmay run: oxplow.collector.sync");
  expect(m.status).toBe("Not approved: it shows and reads, but can't act");
  expect(m.approveTitle).toContain("with your rights");
  expect(m.approveTitle).toContain("oxplow.collector.sync");
  expect(m.approveTitle).toContain("every file of its bundle");
});

test("programRow shows a job with the tools its model may call", () => {
  const job = (commands: string[], scopes: string[]) =>
    programRow({
      kind: "job",
      name: "triage/review",
      program: "ai.complete_typed",
      args: [],
      env: [],
      credentials: [],
      network: [],
      scopes,
      commands,
      tree: ".oxplow/extensions/triage",
      remote: false,
      approved: false,
    });
  const m = job(["triage.todos.lookup"], ["ai.compute", "sql.read"]);
  expect(m.label).toBe("Job triage/review");
  expect(m.command).toBe("ai.complete_typed\nits model may call: triage.todos.lookup, query_sql");
  expect(m.status).toBe("Not approved: queued work waits");
  expect(m.approveTitle).toContain("lets the model call triage.todos.lookup, query_sql");
  expect(job([], ["ai.compute"]).command).toBe("ai.complete_typed");
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
  expect(m.showsSource).toBe(true);
  expect(m.command).toBe("effects/verify.star, part of oxplow-bundled (comes with oxplow)");
  expect(m.approveTitle).toContain("comes with oxplow");
  expect(m.approveTitle).toContain("A new oxplow that changes it asks again");
});

// An AI provider written as a script carries the person's key and
// prompts: its row says where it sends them, offers the script to read,
// and says a new oxplow that changes a shipped one asks again.
test("programRow shows an AI provider script, what it sends to, and its script", () => {
  const m = programRow({
    kind: "ai-provider",
    name: "oxplow-foundation/anthropic",
    program: "bundled:oxplow-foundation/providers/anthropic.star",
    args: [],
    env: [],
    credentials: [],
    network: ["https://api.anthropic.com"],
    commands: [],
    scopes: [],
    tree: "bundled:oxplow-foundation",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("AI provider oxplow-foundation/anthropic");
  expect(m.showsSource).toBe(true);
  expect(m.command).toContain("providers/anthropic.star, part of oxplow-foundation (comes with oxplow)");
  expect(m.command).toContain("sends to: https://api.anthropic.com");
  expect(m.status).toBe("Not approved: calls through it fail");
  expect(m.approveTitle).toContain("your key");
  expect(m.approveTitle).toContain("A new oxplow that changes it asks again");
});

// An effort policy written as a script reacts to core's events by
// composing commands: its row says what it reads, offers the script to
// read, and asks again when any file of its extension changes.
test("programRow shows an effort policy script, what it reads, and its script", () => {
  const m = programRow({
    kind: "effort-policy",
    name: "acme/tidy",
    program: ".oxplow/extensions/acme/policies/tidy.star",
    args: [],
    env: [],
    credentials: [],
    network: [],
    commands: [],
    scopes: ["sql.read"],
    tree: ".oxplow/extensions/acme",
    remote: false,
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Effort policy acme/tidy");
  expect(m.showsSource).toBe(true);
  expect(m.command).toContain(".oxplow/extensions/acme/policies/tidy.star");
  expect(m.command).toContain("reads oxplow's: sql.read");
  expect(m.status).toBe("Not approved: choosing it opens and closes nothing");
  expect(m.approveTitle).toContain("every file in .oxplow/extensions/acme");
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
  const script = { kind: "ai-provider", name: "g", program: "p", args: [], env: [], network: [], approved: false } as never;
  expect(canApprove(script, undefined)).toBe(true);
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
    tree: ".oxplow/extensions/notes",
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


// Settings → Data → Programs shows what the grant doesn't hold, when it was
// given or taken back, and offers to take it back (tsk: grants).
const when = (iso: string) => `<${iso}>`;
const program = (over: Record<string, unknown> = {}) =>
  ({ kind: "ai-provider", name: "tests.parse", program: "tools/parse.star", args: [], env: [], network: [], approved: true, ...over }) as Parameters<typeof programRow>[0];

test("a program row says what isn't held here, worded as the review line says it", () => {
  expect(programRow(program()).unenforced).toBeNull();
  expect(programRow(program({ unenforced: [] })).unenforced).toBeNull();
  const m = programRow(
    program({ unenforced: ["a program reads and writes files as you", "network isn't confined on this OS"] }),
  );
  expect(m.unenforced).toBe("not held here: a program reads and writes files as you; network isn't confined on this OS");
});

test("a program row says when it was approved, or revoked", () => {
  expect(programRow(program({ grantedAt: "2026-10-09T10:00:00Z" }), when).grant).toBe("Approved <2026-10-09T10:00:00Z>");
  const revoked = programRow(program({ approved: false, grantedAt: "2026-10-09T10:00:00Z", revokedAt: "2026-10-10T08:00:00Z" }), when);
  expect(revoked.grant).toBe("Revoked <2026-10-10T08:00:00Z>");
  expect(programRow(program({ approved: false }), when).grant).toBeNull();
  expect(programRow(program(), when).grant).toBeNull();
});

test("only an approved program can be revoked, and the ask names what stops", () => {
  const m = programRow(program());
  expect(m.canRevoke).toBe(true);
  expect(m.revokeAsk).toContain("AI provider tests.parse");
  expect(m.revokeAsk).toMatch(/until you approve it again/);
  expect(programRow(program({ approved: false })).canRevoke).toBe(false);
});

test("an extension's exec or model-calling collector can be revoked once approved", () => {
  const listing = (over: Record<string, unknown>) =>
    ({ owner: "acme", spec: { id: "pull", entry: "pull.sh" }, approved: true, version: "abc", ...over }) as unknown as CollectorListing;
  expect(collectorRevoke(listing({}))?.ask).toContain("acme/pull");
  // A derived collector that calls no model needs no approval: nothing to take back.
  expect(collectorRevoke(listing({ version: null }))).toBeNull();
  expect(collectorRevoke(listing({ approved: false }))).toBeNull();
});
