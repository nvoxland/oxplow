import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, render, waitFor } from "@testing-library/react";
import { fireEvent } from "@testing-library/react";

// tsk248: the launcher offers two distinct doors — "New Project…" creates
// `.oxplow/` in a plain folder, "Open Project…" only opens a folder that is
// already a project. Neither may do the other's job, so this test drives both
// buttons and asserts each lands on its own command.
//
// Only the launcher's commands are overridden on the bindings module (the
// reals are spread so sibling tests keep the full surface); `pickFolder` is
// stubbed to answer with a fixed path instead of opening a native dialog.

const realTransport = await import("../tauri-bridge/transport.js");
const realBindings = await import("../tauri-bridge/generated/bindings.js");
const realDialog = await import("../tauri-bridge/nativeDialog.js");

const ok = <T,>(data: T) => ({ status: "ok" as const, data });

const created: string[] = [];
const opened: [string, boolean][] = [];
let picked: string | null = "/tmp/some-folder";

mock.module("../tauri-bridge/transport.js", () => ({
  ...realTransport,
  listen: async () => () => {},
}));

mock.module("../tauri-bridge/nativeDialog.js", () => ({
  ...realDialog,
  pickFolder: async () => picked,
}));

mock.module("../tauri-bridge/generated/bindings.js", () => ({
  ...realBindings,
  commands: {
    ...realBindings.commands,
    listRecentProjects: async () => ok([]),
    createProject: async (path: string) => {
      created.push(path);
      return ok(null);
    },
    openProject: async (path: string, newWindow: boolean) => {
      opened.push([path, newWindow]);
      return ok(null);
    },
  },
}));

const { Launcher } = await import("./Launcher.js");

afterEach(() => {
  cleanup();
  created.length = 0;
  opened.length = 0;
  picked = "/tmp/some-folder";
});

test("New Project creates the picked folder as a project", async () => {
  const { getByTestId } = render(<Launcher />);
  await waitFor(() => getByTestId("launcher-empty"));

  fireEvent.click(getByTestId("launcher-new-project"));

  await waitFor(() => expect(created).toEqual(["/tmp/some-folder"]));
  expect(opened).toEqual([]);
});

test("Open Project opens the picked folder and never creates one", async () => {
  const { getByTestId } = render(<Launcher />);
  await waitFor(() => getByTestId("launcher-empty"));

  fireEvent.click(getByTestId("launcher-open-project"));

  await waitFor(() => expect(opened).toEqual([["/tmp/some-folder", false]]));
  expect(created).toEqual([]);
});

test("a cancelled folder picker is a no-op on both doors", async () => {
  picked = null;
  const { getByTestId } = render(<Launcher />);
  await waitFor(() => getByTestId("launcher-empty"));

  fireEvent.click(getByTestId("launcher-new-project"));
  fireEvent.click(getByTestId("launcher-open-project"));

  await waitFor(() => expect(created).toEqual([]));
  expect(opened).toEqual([]);
});
