import { expect, test } from "bun:test";

import type { DataEntity } from "../tauri-bridge/generated/bindings.js";
import { entityRows, entitySummary, programRow } from "./dataSectionModel.js";

const entity = (name: string, owner: string, kind: string, rows: number | null): DataEntity => ({
  name,
  owner,
  kind,
  rows,
  description: `${name} rows`,
});

test("entityRows puts core first, formats counts and flags unsynced entities", () => {
  const rows = entityRows([
    entity("v_github_pr", "github", "declared", null),
    entity("v_task", "core", "sql", 12345),
    entity("v_commit", "core", "sql", 0),
    entity("v_linear_issue", "linear", "entity", 7),
  ]);
  expect(rows.map((r) => r.name)).toEqual(["v_commit", "v_task", "v_github_pr", "v_linear_issue"]);
  expect(rows[1]!.rows).toBe(new Intl.NumberFormat().format(12345));
  expect(rows[0]!.rows).toBe("0");
  expect(rows[2]!.rows).toBe("Not synced yet");
  expect(rows[2]!.available).toBe(false);
  expect(rows[3]!.rows).toBe("7");
  expect(entitySummary(rows)).toBe("4 entities · 2 from extensions");
  expect(entitySummary(rows.slice(0, 1))).toBe("1 entity");
});

test("a model whose count didn't come back reads as a dash, not zero", () => {
  expect(entityRows([entity("v_task", "core", "sql", null)])[0]!.rows).toBe("—");
});

test("programRow says what runs and whether it will", () => {
  const m = programRow({ kind: "plugin", name: "acme.parse", program: "tools/parse.sh", args: ["--x"], env: [], approved: false });
  expect(m.label).toBe("Collection plugin acme.parse");
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
    name: "tracker/linear",
    program: "oxplow/extensions/tracker/bin/provider",
    args: ["--stdio"],
    env: ["LINEAR_URL"],
    credentials: ["token"],
    network: ["api.linear.app"],
    tree: "oxplow/extensions/tracker",
    approved: false,
    version: "abc",
  });
  expect(m.label).toBe("Provider tracker/linear");
  expect(m.command).toBe(
    "oxplow/extensions/tracker/bin/provider --stdio\nenv: LINEAR_URL\ncredentials: token\nreaches: api.linear.app",
  );
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("every file in oxplow/extensions/tracker");
});

import { canApprove, providerEffectLines } from "./dataSectionModel.js";
import type { ProviderEffect } from "../tauri-bridge/generated/bindings.js";

// P6b.E3: a provider's Approve waits for what approving would change; the
// change reads as lines — everything new at first, then what differs.
test("a provider's approve waits for its declaration diff", () => {
  const provider = { kind: "provider", name: "tracker/fake", program: "p", args: [], env: [], approved: false } as never;
  expect(canApprove(provider, undefined)).toBe(false);
  expect(canApprove(provider, "loading")).toBe(false);
  expect(canApprove(provider, { change: "added" } as ProviderEffect)).toBe(true);
  expect(canApprove(provider, { error: "no provider.json" })).toBe(false);
  const collector = { kind: "collector", name: "g", program: "p", args: [], env: [], approved: false } as never;
  expect(canApprove(collector, undefined)).toBe(true);
});

test("a provider's declaration diff reads as lines", () => {
  const grants = (hosts: string[]) => ({ entry: "bin/p", runtime: "exec", args: [], hosts, credentials: ["token"], env: [] });
  const first: ProviderEffect = {
    id: "fake",
    capability: "work_items",
    change: "added",
    before: null,
    after: grants(["api.example.com"]),
    commands: [
      { name: "create", change: "added", before: null, after: { confirm: "never" } },
      { name: "delete", change: "added", before: null, after: { confirm: "destructive" } },
    ],
    tools: [],
    featuresBefore: null,
    featuresAfter: { comments: true },
    firstDifference: null,
  };
  expect(providerEffectLines(first)).toEqual([
    "Added — runs bin/p · reaches api.example.com · reads token",
    "Commands: create, delete (destructive)",
  ]);
  const changed: ProviderEffect = {
    ...first,
    change: "changed",
    before: grants([]),
    commands: [
      { name: "create", change: "unchanged", before: {}, after: {} },
      { name: "archive", change: "added", before: null, after: { confirm: "destructive" } },
    ],
    featuresBefore: { comments: true },
  };
  expect(providerEffectLines(changed)).toEqual(["Now reaches api.example.com (was none)", "Command `archive` added (destructive)"]);
  expect(providerEffectLines({ ...changed, change: "unchanged", before: grants(["api.example.com"]), commands: [] })).toEqual([
    "Nothing changed since it was last approved.",
  ]);
});
