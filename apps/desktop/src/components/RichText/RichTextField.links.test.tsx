import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

import { setResources, type Resource } from "../../resources.js";
import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { pageUri, resourceUri, workItemUri } from "../../uri/uri.js";
import { RichTextField } from "./RichTextField.js";

// tsk976: a link in an editable body opens its page whatever it names —
// a task, an extension's ref — as it does in a read-only one (both through
// `linkRef` and `openRef`).

const ITEM = workItemUri("oxplow", "tsk1");
const PR = resourceUri("acme/pr", { id: 12 });

const ACME_PR: Resource = {
  resource: "acme/pr",
  extension: "acme",
  label: "Pull request",
  params: { id: { required: true, pattern: "^\\d+$" } },
  additional: false,
  resolve: "pull_request",
  title: null,
  searchable: null,
  icon: "git-pull-request",
  wikilink: null,
  page: pageUri("acme/pr"),
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
      <RichTextField value={`See [tsk1](${ITEM}) and [pr 12](${PR}).`} onCommit={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  for (const href of [ITEM, PR]) {
    const anchor = view.container.querySelector(`a[href="${href}"]`);
    expect(anchor).not.toBeNull();
    fireEvent.click(anchor!);
  }
  expect(opened).toEqual([ITEM, PR]);
});
