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
