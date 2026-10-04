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
/** What a sign-in's start waits on (resolved unless a test holds it). */
let signInGate: Promise<void> = Promise.resolve();
const added: Array<[string, string, string]> = [];
const removed: string[] = [];
const offHere: string[] = [];
/** Further instances the listing returns after the provider's own. */
let more: unknown[] = [];
const browsed: string[] = [];
/** The sign-in's steps, in order (P10: the shell listens, the core begins,
 *  the browser opens, each redirect goes to the core, the browser hears). */
const steps: string[] = [];
/** Whether this window is the desktop app (only it can sign in). */
let shellPresent = true;
/** What each wait for a redirect returns, in turn (then none comes). */
let redirects: string[] = [];
/** When set, a wait for a redirect fails with this (the shell's listener
 *  broke). */
let awaitFails: string | null = null;
/** How many listeners the shell opened: each one's id. */
let listens = 0;
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
  listProviderInstances: async () => [instance, ...more],
  addProviderInstance: async (inst: string, provider: string, scope: string) => {
    added.push([inst, provider, scope]);
    more = [{ ...instance, instance: inst, instanceId: inst.split("/")[1], scope, enabled: false }];
    return [instance, ...more];
  },
  turnOffProviderInstanceHere: async (inst: string) => {
    offHere.push(inst);
    more = [{ ...instance, instance: inst, instanceId: inst.split("/")[1], scope: "project", overridden: true, enabled: false }];
    return [instance, ...more];
  },
  removeProviderInstance: async (inst: string) => {
    removed.push(inst);
    more = [];
    return [instance];
  },
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
  canCatchSignInRedirect: () => shellPresent,
  listenForSignInRedirect: async (port: number | null) => {
    listens += 1;
    steps.push(`listen ${port}`);
    return { id: listens, port: 5555 };
  },
  stopSignInRedirect: async (id: number) => {
    steps.push(`stop ${id}`);
  },
  beginOauthSignIn: async (inst: string, name: string, port: number) => {
    signIns.push([inst, name]);
    steps.push(`begin ${inst} ${name} ${port}`);
    await signInGate;
    return { url: "https://auth.example.com/authorize?state=abc", signIn: 40 + signIns.length };
  },
  cancelOauthSignIn: async (_inst: string, _name: string, signIn: number) => {
    steps.push(`cancel ${signIn}`);
  },
  openInSystemBrowser: async (url: string) => {
    browsed.push(url);
    steps.push(`open ${url}`);
  },
  awaitSignInRedirect: (id: number) => {
    steps.push(`await ${id}`);
    if (awaitFails !== null) return Promise.reject(new Error(awaitFails));
    const next = redirects.shift();
    return next === undefined ? new Promise<string>(() => {}) : Promise.resolve(next);
  },
  completeOauthSignIn: async (_inst: string, _name: string, redirect: string) => {
    steps.push(`complete ${redirect}`);
    if (redirect.includes("lost")) throw new Error("the connection to the daemon was lost");
    return redirect.includes("forged")
      ? { outcome: "not_this_sign_in", reason: "this isn't the sign-in oxplow started" }
      : { outcome: "signed_in" };
  },
  answerSignInRedirect: async (id: number, outcome: { outcome: string }) => {
    steps.push(`answer ${id} ${outcome.outcome}`);
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

afterEach(async () => {
  // Unmounted first: a row's sign-in under way stops as it goes, and that
  // must land before the record is cleared.
  cleanup();
  await new Promise((r) => setTimeout(r, 0));
  ran.length = 0;
  credentialSaves.length = 0;
  signIns.length = 0;
  added.length = 0;
  removed.length = 0;
  offHere.length = 0;
  more = [];
  browsed.length = 0;
  steps.length = 0;
  listens = 0;
  redirects = [];
  awaitFails = null;
  shellPresent = true;
  instance.credentials = STATIC_CREDENTIALS;
  active = null;
  replacementsOff = [];
  replacements = [];
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
    { id: "tracker/work_item.board", extension: "tracker", target: "work_item.board", capability: "work_items", lensId: "tracker/board", label: "board" },
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
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN"));
  expect(row.querySelector("input")).toBeNull();
  expect(row.textContent).toContain("Not signed in");
  expect(view.queryByTestId("sign-out-tracker/fake-FAKE_TOKEN-trigger")).toBeNull();
  // Someone else's redirect comes first: refused, and the wait goes on.
  redirects = ["/callback?code=x&state=forged", "/callback?code=c&state=abc"];
  fireEvent.click(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  await waitFor(() => expect(steps.at(-1)).toBe("answer 1 signed_in"));
  expect(steps).toEqual([
    "listen null",
    "begin tracker/fake FAKE_TOKEN 5555",
    "open https://auth.example.com/authorize?state=abc",
    "await 1",
    "complete /callback?code=x&state=forged",
    "answer 1 not_this_sign_in",
    "await 1",
    "complete /callback?code=c&state=abc",
    "answer 1 signed_in",
  ]);
  // Signed in: the row stops waiting (oxplow's news then re-reads it).
  await waitFor(() => expect(row.textContent).not.toContain("Finish signing in in your browser"));

  // It came to nothing: why, on the row.
  const hear = (error: string | null) =>
    act(() => {
      for (const l of [...listeners]) l({ kind: "credentialChanged", instance: "tracker/fake", name: "FAKE_TOKEN", signIn: null, error });
    });
  hear("the sign-in was refused: access_denied");
  await waitFor(() => expect(row.textContent).toContain("the sign-in was refused: access_denied"));
  expect(row.textContent).not.toContain("Finish signing in");

  // It worked: the instances are read again.
  instance.credentials = [{ name: "FAKE_TOKEN", set: true, signIn: { state: "signed_in", until: null }, redirectPort: null }];
  hear(null);
  await waitFor(() => expect(view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN").textContent).toContain("Signed in"));
  expect(view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN").textContent).not.toContain("access_denied");
  expect(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN").textContent).toBe("Sign in again");
  // Signing out forgets the token.
  fireEvent.click(view.getByTestId("sign-out-tracker/fake-FAKE_TOKEN-trigger"));
  fireEvent.click(view.getByTestId("sign-out-tracker/fake-FAKE_TOKEN-confirm"));
  await waitFor(() => expect(credentialSaves).toEqual([["tracker/fake", "FAKE_TOKEN", null]]));
});

// P9.B6: another instance of a provider is added by name and scope — the
// project's, or the person's own in every project — and a named one can
// be removed; a provider's own instance can't.
test("an instance is added by name and scope, and a named one removed", async () => {
  const view = render(<IntegrationsSection />);
  const form = await waitFor(() => view.getByTestId("integrations-add-instance"));
  const name = view.getByTestId("integrations-add-name") as HTMLInputElement;
  const add = view.getByTestId("integrations-add-submit") as HTMLButtonElement;
  expect(add.disabled).toBe(true);
  expect(view.queryByTestId("integration-remove-tracker/fake-trigger")).toBeNull();

  fireEvent.change(name, { target: { value: "Fake Two" } });
  expect(add.disabled).toBe(true);
  expect(form.textContent).toContain("lowercase letters, digits and underscores");
  // Escape clears what was typed.
  fireEvent.keyDown(name, { key: "Escape" });
  expect(name.value).toBe("");

  fireEvent.change(name, { target: { value: "fake_two" } });
  fireEvent.click(view.getByTestId("integrations-add-scope-global"));
  expect(add.disabled).toBe(false);
  fireEvent.submit(form);
  await waitFor(() => expect(added).toEqual([["tracker/fake_two", "fake", "global"]]));
  await waitFor(() => view.getByTestId("integration-row-tracker/fake_two"));
  expect(name.value).toBe("");

  fireEvent.click(view.getByTestId("integration-remove-tracker/fake_two-trigger"));
  fireEvent.click(view.getByTestId("integration-remove-tracker/fake_two-confirm"));
  await waitFor(() => expect(removed).toEqual(["tracker/fake_two"]));
  await waitFor(() => expect(view.queryByTestId("integration-row-tracker/fake_two")).toBeNull());
});

// tsk824: where a credential signs in is part of what a person approves, so
// an unapproved provider's Sign in is off (the core refuses it too).
test("Sign in is off while the provider isn't approved", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  const approved = instance.approved;
  instance.approved = false;
  try {
    const view = render(<IntegrationsSection />);
    const button = (await waitFor(() => view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"))) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.title).toContain("Approve");
    fireEvent.click(button);
    expect(signIns).toEqual([]);
  } finally {
    instance.approved = approved;
  }
});

// P10: the redirect comes back to the person's machine, where only the
// desktop app can listen: a plain browser can't sign in, and says why.
test("Sign in is off without the desktop app", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  shellPresent = false;
  const view = render(<IntegrationsSection />);
  const button = (await waitFor(() => view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"))) as HTMLButtonElement;
  expect(button.disabled).toBe(true);
  expect(button.title).toContain("desktop app");
  fireEvent.click(button);
  expect(steps).toEqual([]);
});

// tsk906: a completion that fails without the core announcing it (the
// connection to a remote daemon lost) shows on the row, and the row stops
// waiting.
test("a failed completion shows on its row", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN"));
  redirects = ["/callback?code=c&state=lost"];
  fireEvent.click(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  await waitFor(() => expect(steps.at(-1)).toBe("answer 1 failed"));
  await waitFor(() => expect(row.textContent).toContain("the connection to the daemon was lost"));
  expect(row.textContent).not.toContain("Finish signing in in your browser");
});

// tsk935: the shell's wait for the redirect failing (its listener broke)
// ends the row's sign-in: it says why and stops waiting.
test("a wait for the redirect that fails shows on its row", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  awaitFails = "the redirect listener stopped";
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN"));
  fireEvent.click(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  await waitFor(() => expect(row.textContent).toContain("the redirect listener stopped"));
  expect(row.textContent).not.toContain("Finish signing in in your browser");
  expect(steps.some((s) => s.startsWith("complete"))).toBe(false);
});

// tsk929: leaving the row cancels its sign-in in the core (nothing of it
// kept), and news of another sign-in of the credential isn't the row's.
test("leaving a sign-in cancels it, and another sign-in's news isn't the row's", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  const view = render(<IntegrationsSection />);
  const row = await waitFor(() => view.getByTestId("sign-in-tracker/fake-FAKE_TOKEN"));
  fireEvent.click(view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  await waitFor(() => expect(row.textContent).toContain("Finish signing in in your browser"));
  act(() => {
    for (const l of [...listeners])
      l({ kind: "credentialChanged", instance: "tracker/fake", name: "FAKE_TOKEN", signIn: 7, error: "a newer sign-in replaced it" });
  });
  expect(row.textContent).toContain("Finish signing in in your browser");
  expect(row.textContent).not.toContain("replaced");
  view.unmount();
  await waitFor(() => expect(steps).toContain("cancel 41"));
});

// tsk905: Sign in again while one is under way: the old listener is
// stopped — and its socket closed — before the new one listens, and the
// new one is known by its own id even on the same port.
test("a new sign-in stops the old listener before it listens", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: 8765 }];
  const view = render(<IntegrationsSection />);
  const button = await waitFor(() => view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"));
  fireEvent.click(button);
  await waitFor(() => expect(steps).toContain("await 1"));
  fireEvent.click(button);
  await waitFor(() => expect(steps).toContain("await 2"));
  const stop = steps.indexOf("stop 1");
  const second = steps.lastIndexOf("listen 8765");
  expect(stop).toBeGreaterThan(-1);
  expect(stop).toBeLessThan(second);
});

// A service with its redirect port registered is listened for there.
test("a registered redirect port is the one the shell listens on", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: 8765 }];
  const view = render(<IntegrationsSection />);
  fireEvent.click(await waitFor(() => view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN")));
  await waitFor(() => expect(steps[0]).toBe("listen 8765"));
});

// tsk826: a second click while a sign-in is starting doesn't start another.
test("Sign in is off while a sign-in starts", async () => {
  instance.credentials = [{ name: "FAKE_TOKEN", set: false, signIn: { state: "not_signed_in" }, redirectPort: null }];
  let release = () => {};
  signInGate = new Promise((resolve) => {
    release = resolve;
  });
  try {
    const view = render(<IntegrationsSection />);
    const button = (await waitFor(() => view.getByTestId("sign-in-button-tracker/fake-FAKE_TOKEN"))) as HTMLButtonElement;
    fireEvent.click(button);
    await waitFor(() => expect(button.disabled).toBe(true));
    fireEvent.click(button);
    expect(signIns).toEqual([["tracker/fake", "FAKE_TOKEN"]]);
    release();
    await waitFor(() => expect(button.disabled).toBe(false));
  } finally {
    signInGate = Promise.resolve();
  }
});

// tsk843: a global instance is turned off in one project from its row —
// the project gets its own entry, off — and Remove on that brings it back.
test("a global instance has Off in this project", async () => {
  more = [{ ...instance, instance: "tracker/fake_shared", instanceId: "fake_shared", scope: "global" }];
  const view = render(<IntegrationsSection />);
  await waitFor(() => view.getByTestId("integration-row-tracker/fake_shared"));
  expect(view.queryByTestId("integration-off-here-tracker/fake")).toBeNull();
  fireEvent.click(view.getByTestId("integration-off-here-tracker/fake_shared"));
  await waitFor(() => expect(offHere).toEqual(["tracker/fake_shared"]));
  await waitFor(() => view.getByTestId("integration-remove-tracker/fake_shared-trigger"));
  expect(view.queryByTestId("integration-off-here-tracker/fake_shared")).toBeNull();
});

