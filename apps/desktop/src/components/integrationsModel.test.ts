import { expect, test } from "bun:test";

import type { ProviderInstanceView } from "../tauri-bridge/generated/bindings.js";
import { integrationRow } from "./integrationsModel.js";

const view = (over: Partial<ProviderInstanceView>): ProviderInstanceView => ({
  instance: "tracker/linear",
  extension: "tracker",
  provider: "linear",
  capability: "work_items",
  enabled: false,
  config: {},
  configSchema: null,
  approved: true,
  credentials: [],
  health: { state: { state: "off" }, consecutiveFailures: 0, lastOkAt: null, meanInvokeMs: null },
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
  expect(choices.map((c) => [c.provider, c.running])).toEqual([
    ["oxplow", true],
    ["linear", true],
  ]);
  expect(activeProviderProblem(choices, "oxplow")).toBeNull();
  expect(activeProviderProblem(choices, "linear")).toBeNull();
  expect(activeProviderProblem(workItemsChoices([view({})]), "linear")).toContain("isn't running");
  expect(activeProviderProblem(choices, "jira")).toContain("No enabled extension declares `jira`");
});

// P7.A3: a collector's line says what its reads delivered and when, and
// a failed read is a problem naming why.
test("collectorLine says what a collector's reads delivered", async () => {
  const { collectorLine } = await import("./integrationsModel.js");
  const base = { name: "work_items", entity: "work_item", status: "ok", error: null, lastReadAt: "2026-10-01T00:00:00Z", records: 3 };
  expect(collectorLine({ ...base, status: "never", records: 0, lastReadAt: null })).toEqual({ text: "work_items: not read yet", problem: false });
  expect(collectorLine(base).text).toBe("work_items: 3 records · last read 2026-10-01T00:00:00Z");
  const failed = collectorLine({ ...base, status: "error", error: "timed out" });
  expect(failed.problem).toBe(true);
  expect(failed.text).toContain("its last read failed (timed out)");
});
