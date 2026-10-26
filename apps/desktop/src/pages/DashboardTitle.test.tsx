import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { parse } from "yaml";

import { DashboardEditContext } from "./dashboardAuthoring.js";
import { DashboardTitle } from "./DashboardTitle.js";

afterEach(cleanup);

const yaml = '# Keep this explanation\ntitle: "Build health" # title comment\ncategory: Dashboards\nview: { grid: [] }\n';

function fixture(write: (input: { spec: string }) => Promise<unknown> = async () => {}) {
  const writes: { extension: string; page: string; spec: string; deploy: true }[] = [];
  const errors: { label: string; message: string }[] = [];
  const saved: string[] = [];
  const spec = { page: "my-pages/build-health", extension: "my-pages", slug: "build-health", yaml, writable: true };
  const view = render(<DashboardEditContext.Provider value={{
    read: async () => spec,
    write: async (input) => { writes.push(input); return write(input); },
    report: (error) => errors.push(error),
  }}>
    <DashboardTitle page={spec.page} title="Build health" onSaved={(title) => saved.push(title)} />
  </DashboardEditContext.Provider>);
  const edit = (title: string) => {
    fireEvent.click(view.getByTestId("dashboard-title-edit"));
    const input = view.getByTestId("dashboard-title-input");
    fireEvent.change(input, { target: { value: title } });
    return input;
  };
  return { view, edit, writes, errors, saved, spec };
}

test("Enter renames once, preserving the latest YAML comments, and disables editing while saving", async () => {
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => { finish = resolve; });
  const f = fixture(async () => pending);
  // A source-only edit after the title was displayed must survive.
  f.spec.yaml += "# Not deployed yet\n";
  const input = f.edit("Release health");
  fireEvent.keyDown(input, { key: "Enter" });
  fireEvent.blur(input);
  await waitFor(() => expect(f.writes.length).toBe(1));
  expect(f.writes[0]).toMatchObject({ extension: "my-pages", page: "build-health", deploy: true });
  expect(f.writes[0].spec).toContain("# Not deployed yet");
  expect(f.writes[0].spec).toContain('# Keep this explanation\ntitle: "Release health" # title comment');
  expect(parse(f.writes[0].spec).title).toBe("Release health");
  expect((f.view.getByTestId("dashboard-title-edit") as HTMLButtonElement).disabled).toBe(true);
  expect(f.saved).toEqual([]);
  await act(async () => { finish(); await pending; });
  await waitFor(() => expect(f.saved).toEqual(["Release health"]));
  expect(f.writes.length).toBe(1);
  expect(f.errors).toEqual([]);
});

test("Escape cancels the title draft without saving, including its blur", () => {
  const f = fixture();
  const input = f.edit("Discard me");
  fireEvent.keyDown(input, { key: "Escape" });
  fireEvent.blur(input);
  expect(f.writes).toEqual([]);
  expect(f.saved).toEqual([]);
  expect(f.view.getByTestId("dashboard-title-edit").textContent).toBe("Build health");
});

test("a failed rename records the error and keeps the current title for retry", async () => {
  const f = fixture(async () => { throw new Error("The source changed before saving"); });
  fireEvent.keyDown(f.edit("Release health"), { key: "Enter" });
  await waitFor(() => expect(f.errors.length).toBe(1));
  expect(f.errors[0]).toMatchObject({ label: "Rename dashboard", message: "The source changed before saving" });
  expect(f.view.getByRole("alert").textContent).toContain("source changed");
  expect(f.view.getByTestId("dashboard-title-edit").textContent).toBe("Build health");
  expect((f.view.getByTestId("dashboard-title-edit") as HTMLButtonElement).disabled).toBe(false);
  expect(f.saved).toEqual([]);
});

test("a source that becomes read-only refuses rename before any write", async () => {
  const f = fixture();
  f.spec.writable = false;
  fireEvent.keyDown(f.edit("Release health"), { key: "Enter" });
  await waitFor(() => expect(f.errors.length).toBe(1));
  expect(f.writes).toEqual([]);
  expect(f.saved).toEqual([]);
  expect(f.view.getByTestId("dashboard-title-edit").textContent).toBe("Build health");
});
