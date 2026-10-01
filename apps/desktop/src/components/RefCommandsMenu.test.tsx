import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// P6b.C4: the nav bar's Commands lists extensions' commands about the
// page's ref and runs one as the person, the ref bound into its input.

const realApi = await import("../api.js");
const realRunCommand = realApi.runCommand;
const ran: Array<[string, unknown]> = [];
mock.module("../api.js", () => ({
  ...realApi,
  listExtensions: async () => [
    {
      name: "tracker",
      enabled: true,
      ui: {
        slots: [],
        commands: [
          { id: "tracker/0", extension: "tracker", group: "fake", command: "fake.comment", label: "Comment in Fake…", about: "work_item", placement: ["menu"], input: { ref: "{{ref}}", body: "+1" } },
          { id: "tracker/1", extension: "tracker", group: "tracker", command: "tracker.sync", label: "Sync", about: "commit", placement: ["menu"], input: { sha: "{{ref.id}}" } },
        ],
        decorators: [],
      },
    },
  ],
  runCommand: async (name: string, input: unknown, ...rest: unknown[]) => {
    if (!name.startsWith("fake.")) return (realRunCommand as (...a: unknown[]) => unknown)(name, input, ...rest);
    ran.push([name, input]);
    return { result: null, audit_id: 1, event_id: null, inverse: null };
  },
}));
const { RefCommandsMenu } = await import("./RefCommandsMenu.js");

afterEach(cleanup);

test("a page's ref gets its kind's commands, run as the person with the ref bound", async () => {
  const view = render(<RefCommandsMenu target={{ ref: "work_item:fake:W-1", streamId: null }} buttonStyle={{}} />);
  fireEvent.click(await waitFor(() => view.getByTestId("page-nav-commands")));
  expect(view.getByTestId("page-nav-commands-menu").textContent).toContain("fake");
  expect(view.queryByTestId("page-nav-command-tracker/1")).toBeNull();
  fireEvent.click(view.getByTestId("page-nav-command-tracker/0"));
  await waitFor(() => expect(ran).toEqual([["fake.comment", { ref: "work_item:fake:W-1", body: "+1" }]]));
});

test("no commands for the page's kind, no menu", async () => {
  const view = render(<RefCommandsMenu target={{ ref: "wiki:notes", streamId: null }} buttonStyle={{}} />);
  await new Promise((r) => setTimeout(r, 20));
  expect(view.queryByTestId("page-nav-commands")).toBeNull();
});
