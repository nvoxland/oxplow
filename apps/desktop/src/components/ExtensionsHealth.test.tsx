import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P7.C3: a disabled contribution shows why; Enable Again runs
// `plugin.enable` as the person; Repair with the Agent fills the agent's
// input with one mention and sends nothing.

const realApi = await import("../api.js");
const ran: Array<{ name: string; input: unknown; confirmed: boolean }> = [];
mock.module("../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown, confirmed: boolean) => {
    ran.push({ name, input, confirmed });
    return { result: null };
  },
}));
const { HealthRow } = await import("./ExtensionsSection.js");
const { subscribeAgentInput } = await import("../agent-input-bus.js");
import type { PluginHealth } from "../pluginHealth.js";

afterEach(() => {
  ran.length = 0;
  cleanup();
});

const disabled: PluginHealth = {
  plugin: "tracker",
  contribution: "issues",
  kind: "collector",
  state: "disabled",
  reason: "3 failures in a row; the last: boom",
  failures: 3,
  lastError: "boom",
  meanMs: null,
  deadLetters: 0,
  fresh: true,
  repairItem: "work_item:oxplow:tsk9",
};

test("a disabled contribution shows its reason", () => {
  const view = render(<HealthRow health={disabled} />);
  expect(view.getByTestId("extension-health-tracker-issues").textContent).toContain(
    "Disabled: 3 failures in a row; the last: boom",
  );
});

test("Enable Again runs plugin.enable through the person's commands", async () => {
  const view = render(<HealthRow health={disabled} />);
  fireEvent.click(view.getByTestId("extension-enable-tracker-issues"));
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toEqual({ name: "plugin.enable", input: { plugin: "tracker", kind: "collector", contribution: "issues" }, confirmed: false });
});

test("Repair with the Agent fills the agent input and runs nothing", () => {
  const inserted: string[] = [];
  const off = subscribeAgentInput((t) => inserted.push(t));
  try {
    const view = render(<HealthRow health={disabled} />);
    fireEvent.click(view.getByTestId("extension-repair-tracker-issues"));
  } finally {
    off();
  }
  expect(inserted).toEqual(["Repair the extension described in [oxplow ref work_item:oxplow:tsk9] — read it first."]);
  expect(ran).toEqual([]);
});

test("a healthy contribution offers neither", () => {
  const view = render(<HealthRow health={{ ...disabled, state: "ok", reason: null, repairItem: null }} />);
  expect(view.queryByTestId("extension-enable-tracker-issues")).toBeNull();
  expect(view.queryByTestId("extension-repair-tracker-issues")).toBeNull();
});
