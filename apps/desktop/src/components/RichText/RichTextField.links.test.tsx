import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

import { setRefKinds, type RefKind } from "../../refKinds.js";
import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { refFromTabId, taskRef } from "../../tabs/pageRefs.js";
import type { TabRef } from "../../tabs/tabState.js";
import { RichTextField } from "./RichTextField.js";

// tsk976: a link in an editable body opens its page whatever it names —
// a task, an extension's ref — as it does in a read-only one (both through
// `linkTarget`).

const ACME_PR: RefKind = {
  kind: "acme_pr",
  extension: "acme",
  label: "Pull request",
  idPattern: "^\\d+$",
  wikilinks: [],
  resolve: "pull_request",
  page: "page:ext.acme.pr",
  icon: "git-pull-request",
};

afterEach(() => {
  cleanup();
  setRefKinds([]);
});

test("a task link and an extension's ref open their pages", async () => {
  setRefKinds([ACME_PR]);
  const opened: TabRef[] = [];
  const nav = { navigate: (ref: TabRef) => opened.push(ref) } as unknown as PageNavigation;
  const view = render(
    <PageNavigationContext.Provider value={nav}>
      <RichTextField value="See [tsk1](work_item:oxplow:tsk1) and [pr 12](acme_pr:12)." onCommit={() => {}} />
    </PageNavigationContext.Provider>,
  );
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
  for (const href of ["work_item:oxplow:tsk1", "acme_pr:12"]) {
    const anchor = view.container.querySelector(`a[href="${href}"]`);
    expect(anchor).not.toBeNull();
    fireEvent.click(anchor!);
  }
  expect(opened.map((r) => r.id)).toEqual([taskRef("tsk1").id, refFromTabId("acme_pr:12")!.id]);
});
