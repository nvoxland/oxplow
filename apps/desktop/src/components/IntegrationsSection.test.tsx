import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P7.A2: the project's work items are filed on one provider, chosen on
// Settings → Integrations; the choice is the person's `config.set` of
// `activeProviders` (oxplow's own is the key unset).

const realApi = await import("../api.js");
const ran: Array<[string, unknown, boolean]> = [];
let active: unknown = null;
const instance = {
  instance: "tracker/fake",
  extension: "tracker",
  provider: "fake",
  capability: "work_items",
  enabled: true,
  config: { team: "core" },
  configSchema: { type: "object", properties: { team: { type: "string" } } },
  approved: true,
  credentials: [],
  health: { state: { state: "ready" }, consecutiveFailures: 0, lastOkAt: null, meanInvokeMs: null },
};
mock.module("../api.js", () => ({
  ...realApi,
  listProviderInstances: async () => [instance],
  effectiveConfig: async () => [{ key: "activeProviders", doc: "", value: active, origin: "default", extension: null, humanOnly: true, schema: {} }],
  subscribeOxplowEvents: () => () => {},
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push([name, input, confirmed]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
}));
const { IntegrationsSection } = await import("./IntegrationsSection.js");

afterEach(() => {
  ran.length = 0;
  active = null;
  cleanup();
});

test("choosing a provider as active runs config.set as the person; oxplow unsets it", async () => {
  const view = render(<IntegrationsSection />);
  const oxplow = (await waitFor(() => view.getByTestId("integrations-active-oxplow"))) as HTMLInputElement;
  expect(oxplow.checked).toBe(true);
  fireEvent.click(view.getByTestId("integrations-active-fake"));
  await waitFor(() =>
    expect(ran).toEqual([["config.set", { key: "activeProviders", value: { work_items: "fake" } }, true]]),
  );
  expect((view.getByTestId("integrations-active-fake") as HTMLInputElement).checked).toBe(true);
  fireEvent.click(view.getByTestId("integrations-active-oxplow"));
  await waitFor(() => expect(ran[1]).toEqual(["config.unset", { key: "activeProviders" }, true]));
});

test("an active provider that isn't running is said so", async () => {
  active = { work_items: "linear" };
  const view = render(<IntegrationsSection />);
  const problem = await waitFor(() => view.getByTestId("integrations-active-problem"));
  expect(problem.textContent).toContain("No enabled extension declares `linear`");
});
