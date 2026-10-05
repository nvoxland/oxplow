import { expect, test } from "bun:test";

import type { ProviderInstanceView } from "../tauri-bridge/generated/bindings.js";
import { canRemoveInstance, integrationRow, newInstanceProblem, providerPrograms, signInLine } from "./integrationsModel.js";

const view = (over: Partial<ProviderInstanceView>): ProviderInstanceView => ({
  instance: "tracker/linear",
  scope: "project",
  overridden: false,
  extension: "tracker",
  provider: "linear",
  instanceId: "linear",
  capability: "work_items",
  enabled: false,
  config: {},
  configSchema: null,
  approved: true,
  credentials: [],
  health: { state: { state: "off" }, consecutiveFailures: 0, lastOkAt: null, meanInvokeMs: null, rateLimitedUntil: null, activity: null },
  collectors: [],
  ...over,
});

test("integrationRow says where an instance stands and what the person can do", () => {
  const off = integrationRow(view({}));
  expect([off.label, off.status, off.enableLabel]).toEqual(["tracker/linear · work items", "Off", "Enable"]);
  expect(off.needsApproval).toBe(false);

  const unapproved = integrationRow(view({ approved: false, enabled: true, health: { ...view({}).health, state: { state: "unapproved" } } }));
  expect(unapproved.status).toBe("Not approved on this machine: approve it under Data → Programs");
  expect(unapproved.needsApproval).toBe(true);

  const unconfigured = integrationRow(
    view({ enabled: true, health: { ...view({}).health, state: { state: "unconfigured", problems: [{ path: "/team", message: "required" }] } } }),
  );
  expect(unconfigured.status).toBe("Config problems: /team required");

  const failing = integrationRow(
    view({ enabled: true, health: { ...view({}).health, consecutiveFailures: 2, state: { state: "failing", errors: ["a", "timed out"] } } }),
  );
  expect(failing.status).toBe("Failing (2 in a row): timed out");
  expect(failing.enableLabel).toBe("Disable");

  const disabled = integrationRow(
    view({ enabled: true, health: { ...view({}).health, state: { state: "disabled", reason: "3 failures in a row; the last: boom" } } }),
  );
  expect(disabled.status).toBe("Disabled: 3 failures in a row; the last: boom");
  expect(disabled.enableLabel).toBe("Enable again");

  const ready = integrationRow(
    view({ enabled: true, health: { state: { state: "ready" }, consecutiveFailures: 0, lastOkAt: "2026-09-30T10:00:00Z", meanInvokeMs: 41.6 } }),
  );
  expect(ready.status).toBe("Ready · ~42 ms a call");
  expect(ready.enableLabel).toBe("Disable");
});

// P7.A2: the project's work items go to one provider; oxplow's own is
// always a choice, and an active provider that can't take them is said so.
test("work-items choices list oxplow then each declared provider, and name a provider that can't file", async () => {
  const { workItemsChoices, activeProviderProblem } = await import("./integrationsModel.js");
  const ready = view({ enabled: true, health: { ...view({}).health, state: { state: "ready" } } });
  const choices = workItemsChoices([ready, view({ instance: "other/linear" })]);
  expect(choices.map((c) => [c.id, c.running])).toEqual([
    ["oxplow", true],
    ["linear", true],
  ]);
  expect(activeProviderProblem(choices, "oxplow")).toBeNull();
  expect(activeProviderProblem(choices, "linear")).toBeNull();
  expect(activeProviderProblem(workItemsChoices([view({})]), "linear")).toContain("isn't running");
  expect(activeProviderProblem(choices, "jira")).toContain("No enabled extension declares `jira`");
});

// P9.B1: a choice is an instance — a second instance of one provider is
// its own, under its own id (what `activeProviders` names).
test("each instance of a provider is its own choice, by instance id", async () => {
  const { workItemsChoices } = await import("./integrationsModel.js");
  const ready = { ...view({}).health, state: { state: "ready" as const } };
  const choices = workItemsChoices([
    view({ enabled: true, health: ready }),
    view({ instance: "tracker/linear_acme", instanceId: "linear_acme", enabled: true, health: ready }),
  ]);
  expect(choices.map((c) => [c.id, c.label])).toEqual([
    ["oxplow", "oxplow's tasks"],
    ["linear", "linear (tracker/linear)"],
    ["linear_acme", "linear_acme (tracker/linear_acme)"],
  ]);
});

// P9.B2: a global instance is the person's on this machine — the row says
// so, and that its program is approved per project.
test("a global instance's row says whose it is", () => {
  const ready = { ...view({}).health, state: { state: "ready" as const } };
  expect(integrationRow(view({ scope: "global", enabled: true, health: ready })).label).toBe(
    "tracker/linear · work items · yours, in every project",
  );
  // A project's own entry replacing a global one is the project's
  // (its credentials too, tsk838).
  expect(integrationRow(view({ scope: "project", overridden: true })).label).toBe(
    "tracker/linear · work items · this project's, replacing yours",
  );
  const unapproved = integrationRow(
    view({ scope: "global", approved: false, enabled: true, health: { ...view({}).health, state: { state: "unapproved" } } }),
  );
  expect(unapproved.status).toBe(
    "Not approved in this project: its program is approved per project, under Data → Programs",
  );
  expect(integrationRow(view({})).label).toBe("tracker/linear · work items");
});

test("a missing instance says why, and what to add", () => {
  const missing = integrationRow(
    view({
      instance: "tracker/acme",
      instanceId: "acme",
      health: { ...view({}).health, state: { state: "missing", reason: "`tracker/acme`: add `provider: linear`" } },
    }),
  );
  expect(missing.status).toBe("`tracker/acme`: add `provider: linear`");
  expect(missing.problem).toBe(true);
});

// P7.A3: a collector's line says what its reads delivered and when, and
// a failed read is a problem naming why.
test("collectorLine says what a collector's reads delivered", async () => {
  const { collectorLine } = await import("./integrationsModel.js");
  const base = { name: "work_items", entity: "work_item", status: "ok", error: null, lastReadAt: "2026-10-01T00:00:00Z", records: 3 };
  expect(collectorLine({ ...base, status: "never", records: 0, lastReadAt: null })).toEqual({ text: "work_items: not read yet", problem: false });
  // In local time, as the rest of the app shows times (tsk1038).
  const { formatShortDateTime } = await import("./format.js");
  expect(collectorLine(base).text).toBe(`work_items: 3 records · last read ${formatShortDateTime("2026-10-01T00:00:00Z")}`);
  const failed = collectorLine({ ...base, status: "error", error: "timed out" });
  expect(failed.problem).toBe(true);
  expect(failed.text).toContain("its last read failed (timed out)");
});

// P7.A4: a running read says what it's doing, and a rate limit until
// when; once that time has passed it isn't mentioned.
test("integrationRow shows a read's progress and a rate limit", () => {
  const now = new Date("2026-10-01T12:00:00Z");
  const ready = (health: Partial<ProviderInstanceView["health"]>) =>
    integrationRow(view({ enabled: true, health: { ...view({}).health, state: { state: "ready" }, ...health } }), now);
  expect(ready({ activity: "issues: page 2 (40%)" }).status).toBe("Ready · issues: page 2 (40%)");
  const limited = ready({ rateLimitedUntil: "2026-10-01T12:05:00Z" });
  expect(limited.status).toContain("Ready · rate limited until");
  expect(limited.problem).toBe(false);
  expect(ready({ rateLimitedUntil: "2026-10-01T11:00:00Z" }).status).toBe("Ready");
});

// P9.B3: a credential the person signs in for says where the sign-in
// stands, and what the button does.
test("signInLine says where a sign-in stands", () => {
  expect(signInLine({ state: "not_signed_in" })).toEqual({ text: "Not signed in", action: "Sign in", signedIn: false, problem: false });
  expect(signInLine({ state: "signed_in", until: null })).toEqual({ text: "Signed in", action: "Sign in again", signedIn: true, problem: false });
  const until = signInLine({ state: "signed_in", until: "2026-10-04T12:00:00Z" });
  expect(until.text).toBe(`Signed in until ${new Date("2026-10-04T12:00:00Z").toLocaleString()}`);
  expect(signInLine({ state: "sign_in_again" })).toEqual({
    text: "Sign in again: the sign-in lapsed or was withdrawn",
    action: "Sign in again",
    signedIn: false,
    problem: true,
  });
});

// P9.B6: another instance of a provider is added by naming it; a named
// one (or a project's replacement of the person's own) can be removed.
test("providerPrograms lists each provider once, whatever its instances", () => {
  const views = [
    view({}),
    view({ instance: "tracker/linear_acme", instanceId: "linear_acme" }),
    view({ instance: "notes/notes", extension: "notes", provider: "notes", instanceId: "notes" }),
  ];
  expect(providerPrograms(views)).toEqual([
    { key: "notes/notes", extension: "notes", provider: "notes" },
    { key: "tracker/linear", extension: "tracker", provider: "linear" },
  ]);
});

test("newInstanceProblem says what's wrong with a new instance's name", () => {
  const views = [view({}), view({ instance: "tracker/linear_acme", instanceId: "linear_acme" })];
  expect(newInstanceProblem(views, "linear_two")).toBeNull();
  expect(newInstanceProblem(views, "")).toBe("");
  expect(newInstanceProblem(views, "Linear-Two")).toContain("lowercase letters, digits and underscores");
  expect(newInstanceProblem(views, "2nd")).toContain("starting with a letter");
  expect(newInstanceProblem(views, "linear_acme")).toBe("`linear_acme` is already an instance");
  expect(newInstanceProblem(views, "linear")).toBe("`linear` is already an instance");
  expect(newInstanceProblem(views, "oxplow")).toBe("`oxplow` is oxplow's own");
});

test("canRemoveInstance: a named instance, a project's replacement, or one whose provider is gone", () => {
  expect(canRemoveInstance(view({}))).toBe(false);
  expect(canRemoveInstance(view({ instance: "tracker/linear_acme", instanceId: "linear_acme" }))).toBe(true);
  expect(canRemoveInstance(view({ scope: "global", overridden: true }))).toBe(true);
  expect(canRemoveInstance(view({ health: { ...view({}).health, state: { state: "missing", reason: "gone" } } }))).toBe(true);
});

// tsk840: an instance oxplow won't run as configured says why, as a problem.
test("a refused instance's row says why it didn't start", () => {
  const refused = integrationRow(
    view({
      enabled: true,
      health: {
        ...view({}).health,
        state: { state: "refused", reason: "the command namespace `notes` is already provider:acme/notes's — give the instance another id" },
      },
    }),
  );
  expect(refused.status).toBe(
    "Not started: the command namespace `notes` is already provider:acme/notes's — give the instance another id",
  );
  expect(refused.problem).toBe(true);
});
