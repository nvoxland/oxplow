import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// tsk899: each Git Dashboard action is wired through its command's spec —
// one that asks arms on the first click and runs only on the confirm, one
// that doesn't runs on the click. The cards are rendered from props, so
// only the spec lookup is stubbed.

const realApi = await import("../api.js");
mock.module("../api.js", () => ({
  ...realApi,
  getCommand: async (name: string) => ({
    name,
    summary: name,
    confirm: name === "oxplow.vcs.merge" || name === "oxplow.git.rebase" ? "destructive" : "never",
  }),
}));

const { MergeReadinessCard, UpstreamCard } = await import("./GitDashboardPage.js");

afterEach(cleanup);

/** Long enough for each button's spec to load. */
const specsLoad = () => new Promise((r) => setTimeout(r, 30));

test("push and pull don't ask: a click runs them", async () => {
  const ran: string[] = [];
  const view = render(
    <UpstreamCard
      data={{
        branch: "main",
        headSha: "abc",
        headSubject: "s",
        headDate: null,
        upstream: "origin/main",
        aheadUpstream: 2,
        behindUpstream: 1,
      }}
      onPush={() => ran.push("push")}
      onPullUpstream={() => ran.push("pull")}
      onFetch={() => ran.push("fetch")}
      isPending={() => false}
    />,
  );
  await specsLoad();
  fireEvent.click(view.getByTestId("git-dashboard-push"));
  fireEvent.click(view.getByTestId("git-dashboard-pull"));
  await waitFor(() => expect(ran).toEqual(["push", "pull"]));
});

test("a merge asks: the first click arms, the confirm runs", async () => {
  const ran: string[] = [];
  const view = render(
    <MergeReadinessCard
      report={{
        base: "main",
        rows: [
          {
            streamId: "str2",
            title: "Feature",
            branch: "feature",
            ahead: 2,
            behind: 0,
            overlappingFiles: [],
            readiness: "clean",
          },
        ],
      }}
      currentBranch="main"
      onMerge={(branch) => ran.push(`merge ${branch}`)}
      isPending={() => false}
    />,
  );
  await specsLoad();
  fireEvent.click(view.getByTestId("git-dashboard-divergence-merge"));
  expect(ran).toEqual([]);
  fireEvent.click(view.getByTestId("git-dashboard-divergence-merge-confirm"));
  expect(ran).toEqual(["merge feature"]);
});
