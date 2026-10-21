import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

import { setResources, type Resource } from "../../resources.js";
import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { RichTextField } from "./RichTextField.js";

// tsk976: a link in an editable body opens its page whatever it names —
// a task, an extension's ref — as it does in a read-only one (both through
// `linkRef` and `openRef`).

const ACME_PR: Resource = {
  kind: "acme_pr",
  extension: "acme",
  label: "Pull request",
  idPattern: "^\\d+$",
  wikilinks: [],
  resolve: "pull_request",
  page: "oxplow:oxplow/page?page=acme/pr&ref=%7Bref%7D",
  icon: "git-pull-request",
};

afterEach(() => {
  cleanup();
  setResources([]);
});

test("a work item link and an extension's ref open their pages", async () => {
  setResources([ACME_PR]);
  const opened: string[] = [];
  const nav = { navigate: () => {}, openRef: (ref: string) => opened.push(ref) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <RichTextField value="See [tsk1](oxplow:oxplow/work_item?id=tsk1&provider=oxplow) and [pr 12](acme_pr:12)." onCommit={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  for (const href of ["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", "acme_pr:12"]) {
    const anchor = view.container.querySelector(`a[href="${href}"]`);
    expect(anchor).not.toBeNull();
    fireEvent.click(anchor!);
  }
  expect(opened).toEqual(["oxplow:oxplow/work_item?id=tsk1&provider=oxplow", "acme_pr:12"]);
});
