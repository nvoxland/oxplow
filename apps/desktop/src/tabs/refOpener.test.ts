import { afterEach, expect, test } from "bun:test";

import { setResources, type Resource } from "../resources.js";
import { commentUri, eventUri, fileUri, pageUri, resourceUri, threadUri, wikiUri, workItemUri } from "../uri/uri.js";
import { extPageRef, fileRef, wikiPageRef, workItemTabRef } from "./pageRefs.js";
import { refTarget, refTargetNow } from "./refOpener.js";

const resource = (name: string, page: string | null, extension = "oxplow"): Resource => ({
  resource: `${extension}/${name}`,
  extension,
  label: name,
  params: { id: { required: true, pattern: null } },
  additional: false,
  resolve: null,
  title: null,
  searchable: null,
  icon: null,
  wikilink: null,
  page,
});

const RESOURCES: Resource[] = [
  resource("work_item", null),
  resource("wiki", null),
  resource("file", null),
  resource("thread", null),
  // An oxplow resource that names a page opens there, with the URI as `ref`.
  resource("comment", pageUri("comments")),
  resource("event", null),
  {
    ...resource("pr", pageUri("acme/pr"), "acme"),
    label: "Pull request",
    resolve: "v_acme_prs",
    icon: "git-pull-request",
    wikilink: { prefix: "pr", param: "id" },
  },
  resource("issue", null, "acme"),
];

afterEach(() => setResources([]));

// Every URI opens by one rule: an extension's resource opens its page with
// the URI as `ref`; every other URI opens as itself, through `refFromTabId`
// (a thread or a stream selects instead of opening a tab).
test("a URI opens where its resource's page says, or as itself", () => {
  setResources(RESOURCES);
  const item = workItemUri("oxplow", "tsk1");
  expect(refTargetNow(item)).toEqual({ tab: workItemTabRef(item) });
  expect(refTargetNow(wikiUri("home"))).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow(fileUri("src/a.ts"))).toEqual({ tab: fileRef("src/a.ts") });
  expect(refTargetNow(threadUri("thr4"))).toEqual({ thread: "thr4" });
  const pr = resourceUri("acme/pr", { id: 12 });
  expect(refTargetNow(pr)).toEqual({ tab: extPageRef("acme", "pr", { ref: pr }) });
});

test("an oxplow resource with a page opens it with the URI as its ref", () => {
  setResources(RESOURCES);
  const comment = commentUri("cmt7");
  expect(refTargetNow(comment)).toEqual({
    tab: expect.objectContaining({ id: pageUri("comments", { ref: comment }), kind: "page" }),
  });
});

test("before the registry is read, a URI opens as itself", () => {
  expect(refTargetNow(wikiUri("home"))).toEqual({ tab: wikiPageRef("home") });
  expect(refTargetNow(threadUri("thr4"))).toEqual({ thread: "thr4" });
});

test("refTarget is refTargetNow, awaited", async () => {
  setResources(RESOURCES);
  expect(await refTarget(threadUri("thr4"))).toEqual({ thread: "thr4" });
});

test("a resource with no page, or one nobody declares, opens nothing", async () => {
  setResources(RESOURCES);
  // An extension's resource with no page has nowhere to open.
  expect("unopenable" in (await refTarget(resourceUri("acme/issue", { id: 3 })))).toBe(true);
  // A resource the registry doesn't list, and text that isn't a URI.
  expect("unopenable" in (await refTarget(resourceUri("gone/kind", { id: 1 })))).toBe(true);
  expect("unopenable" in (await refTarget("gone_kind:1"))).toBe(true);
  // An oxplow resource that is no tab opens nothing either.
  expect("unopenable" in (await refTarget(eventUri("00000000-0000-0000-0000-000000000000")))).toBe(true);
});
