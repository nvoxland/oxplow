import { describe, expect, test } from "bun:test";
import type { CollectorListing, Extension } from "../tauri-bridge/generated/bindings.js";
import {
  collectorRan,
  collectorRowModel,
  extensionCredentials,
  extensionRowModel,
  lastRuns,
  reviewModel,
  type RunRow,
} from "./extensionRowModel.js";

const ext = (over: Partial<Extension> = {}): Extension => ({
  name: "review",
  description: "Review lenses",
  path: "oxplow/extensions/review",
  errors: [],
  lenses: [],
  source: null,
  collectors: [],
  origin: "project",
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

  test("a local extension can't be updated and says it lives in the repo", () => {
    const m = extensionRowModel(ext());
    expect(m.canUpdate).toBe(false);
    expect(m.origin).toBe("In this repo");
    expect(m.lensCount).toBe(0);
    expect(m.healthy).toBe(true);
  });

  test("an installed extension shows its url, ref and short sha and can update", () => {
    const m = extensionRowModel(
      ext({ source: { git: "https://github.com/acme/lenses", gitRef: "v2", sha: "0123456789abcdef0123456789abcdef01234567" } }),
    );
    expect(m.canUpdate).toBe(true);
    expect(m.origin).toBe("https://github.com/acme/lenses @ v2 (0123456)");
  });

  test("a bundled extension says it ships with oxplow and can't be updated", () => {
    const m = extensionRowModel(ext({ origin: "bundled" }));
    expect(m.origin).toBe("Ships with oxplow");
    expect(m.canUpdate).toBe(false);
  });

  test("a disabled extension says so and can be turned back on", () => {
    const m = extensionRowModel(ext({ enabled: false }));
    expect(m.enabled).toBe(false);
    expect(m.toggleLabel).toBe("Enable");
    expect(m.disabledNote).toBe("Off for this project (extensions.disabled in .oxplow/project.yaml).");
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

describe("reviewModel", () => {
  const collector = (over: Record<string, unknown> = {}) =>
    ({
      id: "gh",
      doc: "",
      runtime: "exec",
      entry: "sync.sh",
      input: null,
      sync: "replace",
      trigger: { kind: "manual" },
      env: [],
      network: ["api.github.com"],
      credentials: ["TOKEN"],
      entities: [],
      ...over,
    }) as unknown as Extension["collectors"][number];
  const review = (over: Partial<Extension> = {}, problems: string[] = []) => ({
    extension: ext({ name: "shared", ...over }),
    git: "https://github.com/acme/lenses",
    gitRef: null,
    sha: "0123456789abcdef0123456789abcdef01234567",
    problems,
    impact: { lenses: [], models: [], collectors: [], providers: [], effects: [], contributions: [], outOfTime: [], lines: [] },
  });

  test("spells out what runs, where it reaches and what it reads", () => {
    const m = reviewModel(
      review({
        lenses: [{} as Extension["lenses"][number], {} as Extension["lenses"][number]],
        collectors: [collector(), collector({ id: "hot", runtime: "starlark", entry: "hot.star", network: [], credentials: [] })],
      }),
    );
    expect(m.from).toBe("https://github.com/acme/lenses (0123456)");
    expect(m.declares).toEqual([
      "2 lenses",
      "Collector gh runs the program sync.sh · reaches api.github.com · reads TOKEN (you'll approve it before it runs)",
      "Collector hot runs hot.star (starlark, sandboxed: no network, files or credentials)",
    ]);
    expect(m.canInstall).toBe(true);
  });

  const described = (kind: string, local: string) =>
    ({ id: { extension: "shared", kind, local } }) as unknown as Extension["contributions"][number];

  test("every other kind of contribution is a row of its own", () => {
    const m = reviewModel(
      review({
        lenses: [{} as Extension["lenses"][number]],
        contributions: [
          described("lens", "todos"),
          described("collector", "gh"),
          described("metric", "acme.todos"),
          described("event_type", "acme.opened"),
          described("custom_component", "chart"),
          described("slot", "effort.review.details-0"),
        ],
      }),
    );
    expect(m.declares).toEqual([
      "1 lens",
      "Metric acme.todos",
      "Event type acme.opened",
      "Custom component chart",
      "Slot effort.review.details-0",
    ]);
  });

  test("load errors block the install; dry-run problems don't", () => {
    const broken = reviewModel({ ...review({ errors: ["extension.yaml: unknown field `bogus`"] }), impact: null });
    expect(broken.canInstall).toBe(false);
    const m = reviewModel(review({}, ["lens shared/x: column `y` isn't in the query result"]));
    expect(m.canInstall).toBe(true);
    expect(m.problems).toHaveLength(1);
  });

});

// Settings → Extensions shows each extension's last run (`v_extension_run`,
// `latest = 1` per contribution): how it ended and when; failed, denied and
// stale in the warning color, with why.
describe("lastRuns", () => {
  const when = (iso: string) => `<${iso}>`;
  const run = (over: Partial<RunRow> = {}): RunRow => ({
    extension: "acme",
    contribution: "acme/collector/pull",
    outcome: "ok",
    error: null,
    finished_at: "2026-10-10T09:00:00Z",
    decisions: null,
    ...over,
  });

  test("a failing contribution isn't hidden behind another's later ok run", () => {
    const got = lastRuns(
      [
        run({ contribution: "acme/collector/pull", outcome: "failed", error: "rate limited", finished_at: "2026-10-10T07:00:00Z" }),
        run({ contribution: "acme/effect/note", outcome: "ok", finished_at: "2026-10-10T09:00:00Z" }),
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
    const at = (c: string, outcome: string, h: number) =>
      run({ contribution: `acme/${c}`, outcome, error: outcome, finished_at: `2026-10-10T0${h}:00:00Z` });
    const worst = (rows: RunRow[]) => lastRuns(rows, when).acme!.text;
    expect(worst([at("collector/a", "failed", 9), at("effect/b", "stale", 5), at("command/c", "denied", 1)])).toContain("denied · command c");
    expect(worst([at("collector/a", "failed", 9), at("effect/b", "stale", 5)])).toContain("stale · effect b");
    expect(worst([at("collector/a", "failed", 3), at("effect/b", "failed", 8)])).toContain("failed · effect b");
  });

  test("an extension's most recent run, named by its contribution", () => {
    const got = lastRuns(
      [
        run({ contribution: "acme/effect/note", finished_at: "2026-10-10T08:00:00Z" }),
        run({ contribution: "acme/collector/pull", finished_at: "2026-10-10T09:00:00Z" }),
        run({ extension: "beta", contribution: "beta/command/ship.it", finished_at: "2026-10-09T09:00:00Z" }),
      ],
      when,
    );
    expect(got.acme).toEqual({
      text: "Last run: ok · collector pull · <2026-10-10T09:00:00Z>",
      reason: null,
      warn: false,
    });
    expect(got.beta!.text).toBe("Last run: ok · command ship.it · <2026-10-09T09:00:00Z>");
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
