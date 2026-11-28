import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// Git ▸'s dialogs: Compare with… picks what to diff a file against, and
// Rollback… what to throw away.

const realApi = { ...(await import("../../api.js")) }; // a copy: `mock.module` patches a loaded module in place
const ran: { name: string; input: unknown; confirmed: boolean }[] = [];
mock.module("../../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown, confirmed = false) => {
    ran.push({ name, input, confirmed });
    const result =
      name === "oxplow.vcs.branches"
        ? [{ name: "main", remote: null }, { name: "topic", remote: null }]
        : name === "oxplow.vcs.log"
          ? { revisions: [{ id: "abc1234def", short_id: "abc1234", subject: "Fix it", author: "N", time: 0 }] }
          : name === "oxplow.vcs.status"
            ? { entries: [{ path: "src/a.ts", status: "modified" }, { path: "src/b.ts", status: "untracked" }, { path: "c.md", status: "modified" }] }
            : null;
    return { result, command_run_id: null, event_id: null, inverse: null };
  },
  querySql: async () => ({ columns: ["name"], rows: [["v1.0"]], truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }),
}));
const { GitDialogs } = await import("./GitDialogs.js");
const { closeGitDialog, openCompareWith, openRollback } = await import("./gitDialogStore.js");

afterEach(() => {
  act(() => closeGitDialog());
  cleanup();
  ran.length = 0;
});
afterAll(() => {
  mock.module("../../api.js", () => realApi);
});

test("Compare with… lists branches, tags and the file's commits; Enter diffs against the one picked", async () => {
  const opened: string[] = [];
  const view = render(<GitDialogs streamId="str1" openRef={(r) => opened.push(r)} />);
  act(() => openCompareWith("src/a.ts"));
  await waitFor(() => view.getByTestId("git-compare-choice-v1.0"));
  expect(view.getByTestId("git-compare-choice-abc1234def").textContent).toContain("Fix it");
  const search = view.getByTestId("git-compare-search");
  fireEvent.change(search, { target: { value: "top" } });
  fireEvent.keyDown(search, { key: "Enter" });
  expect(opened).toHaveLength(1);
  expect(opened[0]).toContain("page=diff");
  expect(opened[0]).toContain("left=git:topic");
  expect(opened[0]).toContain("right=working");
  expect(view.queryByTestId("git-compare-dialog")).toBeNull();
});

test("Rollback… checks what was asked, and throws away what is checked, confirmed by the dialog", async () => {
  const view = render(<GitDialogs streamId="str1" openRef={() => {}} />);
  act(() => openRollback(["src/a.ts", "src/b.ts", "c.md"], ["src/a.ts", "src/b.ts"]));
  await waitFor(() => view.getByTestId("git-rollback-check-src/a.ts"));
  expect((view.getByTestId("git-rollback-check-c.md") as HTMLInputElement).checked).toBe(false);
  expect(view.getByTestId("git-rollback-files").textContent).toContain("deleted: it is untracked");
  fireEvent.click(view.getByTestId("git-rollback-check-src/b.ts"));
  expect(view.getByTestId("git-rollback-run").textContent).toBe("Rollback 1 File");
  fireEvent.click(view.getByTestId("git-rollback-run"));
  await waitFor(() => expect(view.queryByTestId("git-rollback-dialog") === null).toBe(true));
  expect(ran.find((r) => r.name === "oxplow.vcs.discard")).toEqual({
    name: "oxplow.vcs.discard",
    input: { stream: "str1", paths: ["src/a.ts"] },
    confirmed: true,
  });
});
