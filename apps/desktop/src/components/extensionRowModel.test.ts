import { describe, expect, test } from "bun:test";
import type { CollectorListing, Extension } from "../tauri-bridge/generated/bindings.js";
import { collectorRan, collectorRowModel, extensionCredentials, extensionRowModel, reviewModel } from "./extensionRowModel.js";

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
  expect(collectorRan({ kind: "modelsChanged", models: ["v_task", "v_collector_run"] })).toBe(true);
  expect(collectorRan({ kind: "modelsChanged", models: ["v_task"] })).toBe(false);
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
    effects: { lenses: [], models: [], collectors: [], providers: [], config: null, lines: [] },
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

  test("load errors block the install; dry-run problems don't", () => {
    const broken = reviewModel({ ...review({ errors: ["extension.yaml: unknown field `bogus`"] }), effects: null });
    expect(broken.canInstall).toBe(false);
    const m = reviewModel(review({}, ["lens shared/x: column `y` isn't in the query result"]));
    expect(m.canInstall).toBe(true);
    expect(m.problems).toHaveLength(1);
  });

});
