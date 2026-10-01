import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";

// R16: a provider's Approve waits for its declaration diff — disabled while
// it loads, and still disabled, with the reason shown, when it failed.

const realApi = await import("../api.js");
let answer: () => Promise<unknown> = () => new Promise(() => {});
mock.module("../api.js", () => ({
  ...realApi,
  listDataEntities: async () => [],
  listSources: async () => [],
  listProjectPrograms: async () => [
    {
      kind: "provider",
      name: "tracker/fake",
      program: "oxplow/extensions/tracker/bin/provider",
      args: [],
      env: [],
      credentials: [],
      network: [],
      tree: "oxplow/extensions/tracker",
      approved: false,
      version: "h1",
    },
  ],
  providerDeclarationEffects: () => answer(),
  subscribeOxplowEvents: () => () => {},
}));
const { DataSection } = await import("./DataSection.js");

afterEach(cleanup);

const approve = (view: ReturnType<typeof render>) =>
  view.container.querySelector('[data-testid^="program-approve-"]') as HTMLButtonElement;

test("Approve stays disabled while the diff loads", async () => {
  answer = () => new Promise(() => {});
  const view = render(<DataSection />);
  await waitFor(() => expect(view.container.textContent).toContain("Comparing its declarations"));
  expect(approve(view).disabled).toBe(true);
});

test("a failed diff is shown, and Approve stays disabled", async () => {
  answer = async () => {
    throw new Error("provider.json doesn't parse");
  };
  const view = render(<DataSection />);
  await waitFor(() => expect(view.container.textContent).toContain("provider.json doesn't parse"));
  expect(view.container.textContent).not.toContain("Comparing its declarations");
  expect(approve(view).disabled).toBe(true);
  expect(approve(view).title).toContain("couldn't compare");
});
