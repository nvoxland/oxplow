import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P7.A2: the project's work items are filed on one provider, chosen on
// Settings → Integrations; the choice is the person's `config.set` of
// `activeProviders` (oxplow's own is the key unset).

const realApi = await import("../api.js");
const ran: Array<[string, unknown, boolean]> = [];
let active: unknown = null;
let replacementsOff: string[] = [];
/** `tracker`'s `ui.replacements`. */
let replacements: unknown[] = [];
const credentialSaves: Array<[string, string, string | null]> = [];
const STATIC_CREDENTIALS: unknown[] = [{ name: "FAKE_TOKEN", set: false, signIn: null }];
const signIns: Array<[string, string]> = [];
const browsed: string[] = [];
const listeners = new Set<(event: Record<string, unknown>) => void>();
const instance = {
  instance: "tracker/fake",
  scope: "project",
  overridden: false,
  extension: "tracker",
  provider: "fake",
  instanceId: "fake",
  capability: "work_items",
  enabled: true,
  config: { team: "core" },
  configSchema: { type: "object", properties: { team: { type: "string" } } },
  approved: true,
  credentials: STATIC_CREDENTIALS,
  health: { state: { state: "ready" }, consecutiveFailures: 0, lastOkAt: null, meanInvokeMs: null, rateLimitedUntil: null, activity: null },
  collectors: [{ name: "work_items", entity: "work_item", status: "ok", error: null, lastReadAt: "2026-10-01T00:00:00Z", records: 3 }],
};
mock.module("../api.js", () => ({
  ...realApi,
  listProviderInstances: async () => [instance],
  effectiveConfig: async () => [
    { key: "activeProviders", doc: "", value: active, origin: "default", extension: null, humanOnly: true, schema: {} },
    { key: "replacementsOff", doc: "", value: replacementsOff, origin: "default", extension: null, humanOnly: true, schema: {} },
  ],
  // Nothing unless a test declares a replacement: this fake outlives the
  // file (a module mock is process-wide).
  listExtensions: async () =>
    replacements.length === 0
      ? []
      : [{ name: "tracker", enabled: true, ui: { slots: [], commands: [], decorators: [], replacements }, lenses: [] }],
  subscribeOxplowEvents: (fn: (event: Record<string, unknown>) => void) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },
  beginOauthSignIn: async (inst: string, name: string) => {
    signIns.push([inst, name]);
    return "https://auth.example.com/authorize?state=abc";
  },
  openInSystemBrowser: async (url: string) => {
    browsed.push(url);
  },
  setInstanceCredential: async (inst: string, name: string, value: string | null) => {
    credentialSaves.push([inst, name, value]);
    return [instance];
  },
  setCredential: async () => {
    throw new Error("a provider's credential is its instance's, not the extension's");
  },
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push([name, input, confirmed]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
}));
const { IntegrationsSection } = await import("./IntegrationsSection.js");

afterEach(() => {
  ran.length = 0;
  credentialSaves.length = 0;
  signIns.length = 0;
  browsed.length = 0;
  instance.credentials = STATIC_CREDENTIALS;
  active = null;
  replacementsOff = [];
  replacements = [];
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

// P7.A3: a running instance's collector shows its last read and records,
// and Sync Now reads it as the person.
test("Sync Now runs provider.sync for that collector", async () => {
  const view = render(<IntegrationsSection />);
  const line = await waitFor(() => view.getByTestId("integration-collector-tracker/fake-work_items"));
  expect(line.textContent).toContain("work_items: 3 records · last read 2026-10-01T00:00:00Z");
  fireEvent.click(view.getByTestId("integration-sync-tracker/fake-work_items"));
  await waitFor(() =>
    expect(ran).toEqual([["provider.sync", { instance: "tracker/fake", collector: "work_items" }, false]]),
  );
});

// P9.A1: a person can keep oxplow's own component where an extension
// replaces it — `replacementsOff`, a person's `config.set` like the active
// provider.
test("a replaced component can be turned back to oxplow's own", async () => {
  const none = render(<IntegrationsSection />);
  await waitFor(() => none.getByTestId("integrations-section"));
  expect(none.queryByTestId("integrations-replacements")).toBeNull();
  cleanup();

  replacements = [
    { id: "tracker/work_item.board", extension: "tracker", target: "work_item.board", capability: "work_items", lensId: "tracker/board" },
  ];
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("integrations-replacement-work_item.board"));
  expect(row.textContent).toContain("tracker");
  const box = view.getByTestId("integrations-replacement-off-work_item.board") as HTMLInputElement;
  expect(box.checked).toBe(false);
  fireEvent.click(box);
  await waitFor(() =>
    expect(ran).toEqual([["config.set", { key: "replacementsOff", value: ["work_item.board"] }, true]]),
  );
  await waitFor(() => expect(box.checked).toBe(true));
  fireEvent.click(box);
  await waitFor(() => expect(ran[1]).toEqual(["config.unset", { key: "replacementsOff" }, true]));
});

// P9.B1: a provider's credential is an instance's own — saved under the
// instance, never the extension.
test("a credential is saved to its instance", async () => {
  const view = render(<IntegrationsSection />);
  const form = await waitFor(() => view.getByTestId("credential-tracker/fake-FAKE_TOKEN"));
  const input = form.querySelector("input") as HTMLInputElement;
  fireEvent.change(input, { target: { value: "s3cret" } });
  fireEvent.submit(form);
  await waitFor(() => expect(credentialSaves).toEqual([["tracker/fake", "FAKE_TOKEN", "s3cret"]]));
});

// P9.B3: a credential the person signs in for is never typed — the row
// has a Sign in button, which opens the provider's page in their browser,
// and says how it went when oxplow hears.
test("a signed-in credential has a Sign in button and no value box", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" } }];
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN"));
  expect(row.querySelector("input")).toBeNull();
  expect(row.textContent).toContain("Not signed in");
  expect(view.queryByTestId("sign-out-tracker/fake-FAKE_TOKEN-trigger")).toBeNull();
  fireEvent.click(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  await waitFor(() => expect(browsed).toEqual(["https://auth.example.com/authorize?state=abc"]));
  expect(signIns).toEqual([["tracker/fake", "FAKE_TOKEN"]]);
  await waitFor(() => expect(row.textContent).toContain("Finish signing in in your browser"));

  // It came to nothing: why, on the row.
  const hear = (error: string | null) =>
    act(() => {
      for (const l of [...listeners]) l({ kind: "credentialChanged", instance: "tracker/fake", name: "FAKE_TOKEN", error });
    });
  hear("the sign-in was refused: access_denied");
  await waitFor(() => expect(row.textContent).toContain("the sign-in was refused: access_denied"));
  expect(row.textContent).not.toContain("Finish signing in");

  // It worked: the instances are read again.
  instance.credentials = [{ name: "FAKE_TOKEN", set: true, signIn: { state: "signed_in", until: null } }];
  hear(null);
  await waitFor(() => expect(view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN").textContent).toContain("Signed in"));
  expect(view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN").textContent).not.toContain("access_denied");
  expect(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN").textContent).toBe("Sign in again");
  // Signing out forgets the token.
  fireEvent.click(view.getByTestId("sign-out-tracker/fake-FAKE_TOKEN-trigger"));
  fireEvent.click(view.getByTestId("sign-out-tracker/fake-FAKE_TOKEN-confirm"));
  await waitFor(() => expect(credentialSaves).toEqual([["tracker/fake", "FAKE_TOKEN", null]]));
});
