import { describe, expect, test } from "bun:test";
import type { CollectorListing, Extension } from "../tauri-bridge/generated/bindings.js";
import {
  collectorRan,
  collectorRowModel,
  extensionCredentials,
  extensionRowModel,
  lastRuns,
  secretCredentials,
  secretRowModel,
  serviceLines,
  type RunRow,
} from "./extensionRowModel.js";

const ext = (over: Partial<Extension> = {}): Extension => ({
  name: "review",
  description: "Review lenses",
  path: ".oxplow/runtime/extensions/review/0123456789abcdef",
  errors: [],
  lenses: [],
  collectors: [],
  origin: "project",
  deployment: { kind: "deployed", hash: "0123456789abcdef" },
  ui: { slots: [], commands: [], decorators: [] },
  enabled: true,
  ...over,
});

describe("extensionRowModel", () => {
  // tsk1039: a lens whose SQL fails passes loading but fails `extension check`;
  // its row shows the check's errors too, once each.
  test("a project extension's row shows what its check found", () => {
    const m = extensionRowModel(ext({ errors: ["extension.yaml: bad"] }), [
      "extension.yaml: bad",
      "lens review/todos: query_sql: no such table: v_tree_facts",
    ]);
    expect(m.errors).toEqual(["extension.yaml: bad", "lens review/todos: query_sql: no such table: v_tree_facts"]);
    expect(m.healthy).toBe(false);
  });

  test("a deployed extension names its deployment", () => {
    const m = extensionRowModel(ext());
    expect(m.origin).toBe("Deployed 0123456");
    expect(m.canDeploy).toBe(false);
    expect(m.lensCount).toBe(0);
    expect(m.healthy).toBe(true);
  });

  test("a bundled extension says it ships with oxplow", () => {
    const m = extensionRowModel(ext({ origin: "bundled", deployment: { kind: "shipped", hash: "0123456789abcdef" } }));
    expect(m.origin).toBe("Ships with oxplow");
  });

  test("a source never deployed offers Deploy and says why it is off", () => {
    const m = extensionRowModel(ext({ enabled: false, deployment: { kind: "undeployed" } }));
    expect(m.origin).toBe("Not deployed");
    expect(m.canDeploy).toBe(true);
    expect(m.disabledNote).toBe("Deploy it to run it.");
  });

  test("a listed extension with no source here is not an error", () => {
    const m = extensionRowModel(ext({ enabled: false, deployment: { kind: "not_here" } }));
    expect(m.origin).toBe("Not on this machine");
    expect(m.canDeploy).toBe(false);
    expect(m.healthy).toBe(true);
    expect(m.disabledNote).toBe("Listed as on, but its source isn't on this machine.");
  });

  test("a disabled extension says so and can be turned back on", () => {
    const m = extensionRowModel(ext({ enabled: false }));
    expect(m.enabled).toBe(false);
    expect(m.toggleLabel).toBe("Enable");
    expect(m.disabledNote).toBe(
      "Off: not in the project's list (.oxplow/project.yaml), or turned off for you (.oxplow/personal.yaml).",
    );
    const on = extensionRowModel(ext());
    expect(on.toggleLabel).toBe("Disable");
    expect(on.disabledNote).toBeNull();
  });

  test("errors mark the row unhealthy", () => {
    expect(extensionRowModel(ext({ errors: ["bad yaml"] })).healthy).toBe(false);
  });
});

describe("collectorRowModel", () => {
  const listing = (over: Partial<CollectorListing> = {}): CollectorListing => ({
    owner: "my-gh",
    spec: {
      id: "gh",
      doc: "",
      runtime: "exec",
      entry: "bin/sync.sh",
      provider: null,
      trigger: { kind: "every", minutes: 10 },
      after: [],
      input: null,
      report: null,
      sync: "replace",
      env: ["GITHUB_TOKEN"],
      network: [],
      credentials: [],
      entities: [],
      facts: [],
    },
    run: null,
    approved: false,
    networkEnforced: true,
    credentials: [],
    ...over,
  });

  test("an unapproved collector asks for approval and says what it will run", () => {
    const m = collectorRowModel(listing());
    expect(m.action).toBe("approve");
    expect(m.actionLabel).toBe("Approve & Run");
    expect(m.actionTitle).toContain("bin/sync.sh");
    expect(m.actionTitle).toContain("GITHUB_TOKEN");
    expect(m.status).toBe("Never run");
    expect(m.trigger).toBe("every 10m");
  });

  // A Starlark collector asks for approval only when it calls a model: each
  // run spends the AI provider's key, on its trigger, as nobody.
  test("an unapproved Starlark collector's approval says it calls a model on its trigger", () => {
    const m = collectorRowModel(
      listing({ spec: { ...listing().spec, runtime: "starlark", entry: "kind.star", env: [] } }),
    );
    expect(m.action).toBe("approve");
    expect(m.actionTitle).toContain("my-gh/kind.star");
    expect(m.actionTitle).toContain("calls a model through your AI provider");
    expect(m.actionTitle).toContain("every 10m");
    expect(m.actionTitle).not.toContain("network");
  });

  test("the approval names the hosts a collector may reach, and whether that's enforced", () => {
    const withHosts = (networkEnforced: boolean) =>
      listing({ networkEnforced, spec: { ...listing().spec, network: ["api.github.com"] } });
    expect(collectorRowModel(withHosts(true)).actionTitle).toContain("reach only api.github.com");
    expect(collectorRowModel(withHosts(false)).actionTitle).toContain("not enforced on this OS");
    expect(collectorRowModel(listing()).actionTitle).toContain("no network access");
  });

  // The project's own collectors (its report collectors among them), listed
  // beside the extensions': owner `project`.
  test("a report collector says when it runs, and syncs without an approval", () => {
    const m = collectorRowModel(
      listing({
        owner: "project",
        approved: true,
        spec: { ...listing().spec, id: "tests.cov", runtime: "jaq", entry: "oxplow:lcov", trigger: { kind: "onRun", run: "test" }, env: [] },
      }),
    );
    expect(m.trigger).toBe("after a test run");
    expect(m.action).toBe("sync");
    expect(m.actionLabel).toBe("Sync Now");
    const analysis = collectorRowModel(
      listing({ owner: "project", approved: true, spec: { ...listing().spec, trigger: { kind: "onRun", run: "analysis" } } }),
    );
    expect(analysis.trigger).toBe("after an analysis run");
  });

  test("a project collector's approval is the Programs row's, not this button's", () => {
    const m = collectorRowModel(listing({ owner: "project", version: "h1" }));
    expect(m.action).toBe("approve-in-programs");
    expect(m.actionLabel).toBe("Approve under Programs");
    expect(m.actionTitle).toMatch(/Programs/);
  });

  test("a collector failures disabled says why and offers Enable Again", () => {
    const m = collectorRowModel(listing({ approved: true, disabled: "failed 3 times in a row: report not found" }));
    expect(m.disabled).toBe("Off on this machine: failed 3 times in a row: report not found");
    expect(m.canEnableAgain).toBe(true);
    expect(collectorRowModel(listing({ approved: true })).disabled).toBeNull();
    expect(collectorRowModel(listing({ approved: true })).canEnableAgain).toBe(false);
  });

  test("an approved collector syncs and summarizes its last run", () => {
    const m = collectorRowModel(
      listing({
        approved: true,
        run: { owner: "my-gh", id: "gh", status: "ok", lastRunAt: "2026-09-27T01:00:00Z", error: null, rowCounts: { pr: 12, review: 3 }, cursor: null, lastEventId: null },
      }),
    );
    expect(m.action).toBe("sync");
    expect(m.actionLabel).toBe("Sync Now");
    expect(m.status).toBe("12 pr · 3 review");
    expect(m.error).toBeNull();
  });

  test("a failed run surfaces the error", () => {
    const m = collectorRowModel(
      listing({
        approved: true,
        run: { owner: "my-gh", id: "gh", status: "error", lastRunAt: "2026-09-27T01:00:00Z", error: "boom", rowCounts: {}, cursor: null, lastEventId: null },
      }),
    );
    expect(m.status).toBe("Failed");
    expect(m.error).toBe("boom");
    expect(collectorRowModel(listing({ spec: { ...listing().spec, trigger: { kind: "manual" } } })).trigger).toBe("manual");
    expect(
      collectorRowModel(
        listing({ spec: { ...listing().spec, trigger: { kind: "on", events: ["snapshot.taken", "vcs.head.moved"], filter: {} } } }),
      ).trigger,
    ).toBe("on snapshot.taken, vcs.head.moved");
  });
  test("unset credentials are listed and flagged; the approval hover names them", () => {
    const m = collectorRowModel(
      listing({
        spec: { ...listing().spec, trigger: { kind: "manual" }, env: [], credentials: ["GH_PAT", "OTHER"] },
        credentials: [
          { name: "GH_PAT", set: true },
          { name: "OTHER", set: false },
        ],
      }),
    );
    expect(m.credentials).toEqual([
      { name: "GH_PAT", set: true },
      { name: "OTHER", set: false },
    ]);
    expect(m.missingCredentials).toBe("Needs OTHER (set it under Extensions).");
    expect(m.actionTitle).toContain("GH_PAT");
  });
});

test("extensionCredentials lists each declared credential once per extension", () => {
  const l = (extension: string, creds: { name: string; set: boolean }[]) =>
    ({ owner: extension, credentials: creds }) as unknown as CollectorListing;
  expect(
    extensionCredentials(
      [
        l("gh", [{ name: "TOKEN", set: true }]),
        l("gh", [{ name: "TOKEN", set: true }, { name: "APP", set: false }]),
        l("other", [{ name: "X", set: false }]),
      ],
      "gh",
    ),
  ).toEqual([
    { name: "APP", set: false },
    { name: "TOKEN", set: true },
  ]);
});

test("a collector run is a v_collector_run change", () => {
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item", "v_collector_run"] })).toBe(true);
  expect(collectorRan({ kind: "modelsChanged", models: ["v_work_item"] })).toBe(false);
  expect(collectorRan({ kind: "tasksChanged" })).toBe(false);
});

describe("lastRuns", () => {
  const when = (iso: string) => `<${iso}>`;
  const run = (over: Partial<RunRow> = {}): RunRow => ({
    extension: "acme",
    contribution: "collector:acme/pull",
    outcome: "ok",
    error: null,
    finished_at: "2026-10-10T09:00:00Z",
    decisions: null,
    ...over,
  });

  test("a failing contribution isn't hidden behind another's later ok run", () => {
    const got = lastRuns(
      [
        run({ contribution: "collector:acme/pull", outcome: "failed", error: "rate limited", finished_at: "2026-10-10T07:00:00Z" }),
        run({ contribution: "effect:acme/note", outcome: "ok", finished_at: "2026-10-10T09:00:00Z" }),
      ],
      when,
    );
    expect(got.acme).toEqual({
      text: "Last run: failed · collector pull · <2026-10-10T07:00:00Z>",
      reason: "rate limited",
      warn: true,
    });
  });

  test("of several warnings, denied beats stale beats failed, then the newest", () => {
    const at = (contribution: string, outcome: string, h: number) =>
      run({ contribution, outcome, error: outcome, finished_at: `2026-10-10T0${h}:00:00Z` });
    const worst = (rows: RunRow[]) => lastRuns(rows, when).acme!.text;
    expect(worst([at("collector:acme/a", "failed", 9), at("effect:acme/b", "stale", 5), at("command:acme.c", "denied", 1)])).toContain("denied · command acme.c");
    expect(worst([at("collector:acme/a", "failed", 9), at("effect:acme/b", "stale", 5)])).toContain("stale · effect b");
    expect(worst([at("collector:acme/a", "failed", 3), at("effect:acme/b", "failed", 8)])).toContain("failed · effect b");
  });

  test("an extension's most recent run, named by its contribution", () => {
    const got = lastRuns(
      [
        run({ contribution: "effect:acme/note", finished_at: "2026-10-10T08:00:00Z" }),
        run({ contribution: "collector:acme/pull", finished_at: "2026-10-10T09:00:00Z" }),
        run({ extension: "beta", contribution: "command:beta.ship.it", finished_at: "2026-10-09T09:00:00Z" }),
      ],
      when,
    );
    expect(got.acme).toEqual({
      text: "Last run: ok · collector pull · <2026-10-10T09:00:00Z>",
      reason: null,
      warn: false,
    });
    expect(got.beta!.text).toBe("Last run: ok · command beta.ship.it · <2026-10-09T09:00:00Z>");
  });

  test("failed, denied and stale are warnings, and say why", () => {
    const one = (r: Partial<RunRow>) => lastRuns([run(r)], when).acme!;
    expect(one({ outcome: "failed", error: "rate limited" })).toMatchObject({ warn: true, reason: "rate limited" });
    expect(one({ outcome: "stale", error: "the extension changed under it" })).toMatchObject({
      warn: true,
      reason: "the extension changed under it",
    });
    for (const quiet of ["ok", "skipped", "cancelled"]) {
      expect(one({ outcome: quiet }).warn).toBe(false);
    }
  });

  test("a denied run's reason comes from the decision that refused", () => {
    const decisions = JSON.stringify([
      { need: "scope", name: "sql.read", decision: "allowed" },
      { need: "host", name: "evil.example", decision: "denied", reason: "it isn't in the grant" },
      { need: "files", decision: "unenforced", reason: "a program reads files as you" },
    ]);
    const got = lastRuns([run({ outcome: "denied", error: "denied", decisions })], when).acme!;
    expect(got.warn).toBe(true);
    expect(got.reason).toBe("host evil.example: it isn't in the grant");
    // With no decision recorded, the run's own error stands.
    expect(lastRuns([run({ outcome: "denied", error: "no grant", decisions: "[]" })], when).acme!.reason).toBe("no grant");
    expect(lastRuns([run({ outcome: "denied", error: null, decisions: "not json" })], when).acme!.reason).toBeNull();
  });
});

describe("serviceLines", () => {
  const listing = [
    { scope: "sql.read", title: "Read the semantic layer", summary: "", approval: false },
    { scope: "tree.read", title: "Read the captured tree", summary: "", approval: false },
    { scope: "ai.compute", title: "Ask a model", summary: "", approval: true },
  ];
  const use = (scope: string, declared: "declared" | "implied" | "undeclared", approval = false) => ({
    scope,
    declared,
    used: true,
    approval,
  });

  test("names each script's services by their titles, and a model call as needing approval", () => {
    expect(
      serviceLines(
        [
          { script: "collector `paths`", entry: "paths.star", services: [use("tree.read", "implied"), use("ai.compute", "declared", true)] },
          { script: "effect `stamp`", entry: "stamp.star", services: [use("sql.read", "declared")] },
        ],
        listing,
      ),
    ).toEqual([
      { script: "Collector paths", text: "Read the captured tree · Ask a model (you approve it)" },
      { script: "Effect stamp", text: "Read the semantic layer" },
    ]);
  });

  test("says when a script uses a service it doesn't declare, and skips scripts with none", () => {
    expect(
      serviceLines(
        [
          { script: "command `note.sum`", entry: "sum.star", services: [use("sql.read", "undeclared")] },
          { script: "effect `quiet`", entry: "quiet.star", services: [] },
        ],
        listing,
      ),
    ).toEqual([{ script: "Command note.sum", text: "Read the semantic layer (not in its needs)" }]);
  });

  test("a service the listing doesn't know is shown by its scope", () => {
    expect(
      serviceLines([{ script: "effect `x`", entry: "x.star", services: [use("future.thing", "declared")] }], listing),
    ).toEqual([{ script: "Effect x", text: "future.thing" }]);
  });
});

describe("secretCredentials", () => {
  const ref = { schemaVersion: 1, id: "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b" } as const;
  const provider = (credentials: unknown[]) =>
    ({ id: "remote", credentials }) as unknown as Extension["providers"][number];

  test("an extension's credentials with a secret reference are listed once per provider", () => {
    const list = secretCredentials(
      ext({
        providers: [
          provider([{ name: "OXPLOW_MCP_BEARER", oauth: null, secretRef: ref }, { name: "PLAIN", oauth: null, secretRef: null }]),
        ],
      }),
    );
    expect(list).toEqual([{ provider: "remote", name: "OXPLOW_MCP_BEARER", reference: ref }]);
  });

  test("an extension with no secret references lists none", () => {
    expect(secretCredentials(ext())).toEqual([]);
  });

  test("the row says whether a value is set, and offers Rotate only once it is", () => {
    expect(secretRowModel(null)).toEqual({ status: "Checking…", setLabel: "Set", canRevoke: false });
    expect(secretRowModel(false)).toEqual({ status: "Not set", setLabel: "Set", canRevoke: false });
    expect(secretRowModel(true)).toEqual({ status: "Set", setLabel: "Rotate", canRevoke: true });
  });
});
