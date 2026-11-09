import { expect, test } from "bun:test";

import { markdownTree, type TreeDeps } from "./markdownTree.js";

// What a component's frame draws markdown from (the kit's
// `renderMarkdown`): the viewer's parse, its link rules and titles, as a
// tree of the elements a frame may draw — no HTML crosses the bridge.

const ITEM = "oxplow:oxplow/work_item?id=tsk7&provider=oxplow";
const deps: TreeDeps = {
  mentionRef: (id) => (id === "tsk7" ? ITEM : null),
  refTitle: async ({ itemRef }) =>
    itemRef === ITEM ? { title: "The item", brokenReason: null } : itemRef ? { title: null, brokenReason: `Broken link: ${itemRef}` } : { title: null, brokenReason: null },
};

test("one line keeps its marks and links, a wikilink by its title, and unwraps blocks", async () => {
  expect(await markdownTree("- Fix **it** in `a.ts`, see [[tsk7]] and [docs](https://example.com/d).", true, deps)).toEqual([
    "Fix ",
    { tag: "strong", children: ["it"] },
    " in ",
    { tag: "code", children: ["a.ts"] },
    ", see ",
    { tag: "a", ref: ITEM, children: ["The item"] },
    " and ",
    { tag: "a", href: "https://example.com/d", children: ["docs"] },
    ".",
  ]);
});

test("a whole body keeps its blocks", async () => {
  expect(await markdownTree("# Why\n\nIt *breaks*.\n\n- one\n- two", false, deps)).toEqual([
    { tag: "h1", children: ["Why"] },
    { tag: "p", children: ["It ", { tag: "em", children: ["breaks"] }, "."] },
    { tag: "ul", children: [{ tag: "li", children: ["one"] }, { tag: "li", children: ["two"] }] },
  ]);
});

test("a broken ref is marked; a script, raw HTML and an image never cross", async () => {
  const missing = "oxplow:oxplow/work_item?id=tsk9&provider=oxplow";
  expect(await markdownTree(`See [gone](${missing}), [x](javascript:alert(1)) <b>bold</b> ![pic](https://example.com/p.png) end`, true, deps)).toEqual([
    "See ",
    { tag: "a", broken: `Broken link: ${missing}`, children: ["gone"] },
    ", x bold end",
  ]);
});
