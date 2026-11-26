import { afterEach, beforeEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { shellStorage } from "../shellStorage.js";
import { HostHome } from "./HostHome.js";
import type { HostProject } from "./hostApi.js";

beforeEach(() => shellStorage.setItem("oxplow.hostToken", "tok"));
afterEach(() => {
  cleanup();
  shellStorage.removeItem("oxplow.hostToken");
});

const project = (over: Partial<HostProject>): HostProject => ({
  id: "prj-1",
  name: "alpha",
  state: "running",
  holds: 1,
  held_by_you: false,
  ...over,
});

// Open holds the project for this device and goes to its page; a project
// this device holds offers Close, which lets go of it.
test("open holds the project and goes to it; close lets go", async () => {
  const holds: Array<[string, boolean]> = [];
  let went = "";
  const list = [project({}), project({ id: "prj-2", name: "beta", held_by_you: true })];
  render(
    <HostHome
      api={{
        listProjects: async () => list,
        setHold: async (_h, _t, id, held) => {
          holds.push([id, held]);
        },
        forgetDevice: async () => {},
      }}
      go={(path) => (went = path)}
    />,
  );
  await waitFor(() => expect(screen.getByTestId("host-project-prj-1")).toBeTruthy());
  expect(screen.queryByTestId("host-project-close-prj-1")).toBeNull();
  fireEvent.click(screen.getByTestId("host-project-open-prj-1"));
  await waitFor(() => expect(went).toBe("/p/prj-1/"));
  fireEvent.click(screen.getByTestId("host-project-close-prj-2"));
  await waitFor(() => expect(holds).toEqual([["prj-1", true], ["prj-2", false]]));
});

test("without a token it asks to pair", () => {
  shellStorage.removeItem("oxplow.hostToken");
  render(<HostHome />);
  expect(screen.getByTestId("pair-screen")).toBeTruthy();
});
