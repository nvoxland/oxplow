import { afterAll, afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";

import { wikiUri } from "../../uri/uri.js";

// A bare ref link shows its title in the editor as in the viewer, while
// the text it stores stays the slug; the caret inside shows the slug to
// edit, and a labelled link keeps its label.

// The title cache is the module's, shared with every other test: stand in
// for its lookup.
const realCache = { ...(await import("../../wikiTitleCache.js")) };
afterAll(() => {
  mock.module("../../wikiTitleCache.js", () => realCache);
});
mock.module("../../wikiTitleCache.js", () => ({
  ...realCache,
  useWikiRef: (slug: string | null | undefined) =>
    slug === "local-snapshots" ? { title: "Local Snapshots", status: "found" } : { title: null, status: "loading" },
}));
const { RichTextField } = await import("./RichTextField.js");
const { getMarkdown } = await import("./richTextExtensions.js");

afterEach(cleanup);

const HREF = wikiUri("local-snapshots");

async function settle() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

test("a bare wiki link shows the page's title, and the stored text stays its slug", async () => {
  const view = render(<RichTextField value={`See [local-snapshots](${HREF}).`} onChange={() => {}} />);
  await settle();
  await waitFor(() => expect(view.container.querySelector("[data-ref-title]")?.textContent).toBe("Local Snapshots"));
  // The link itself is still there, with the slug as its text.
  expect(view.container.querySelector(`a[href="${HREF}"]:not([data-ref-title])`)?.textContent).toBe("local-snapshots");
});

test("the caret inside the link shows its slug to edit", async () => {
  const view = render(<RichTextField value={`See [local-snapshots](${HREF}).`} onChange={() => {}} />);
  await settle();
  await waitFor(() => expect(view.container.querySelector("[data-ref-title]")).not.toBeNull());
  // Tiptap hangs its editor on the editable element.
  const editor = (view.container.querySelector(".ProseMirror") as unknown as { editor: import("@tiptap/core").Editor }).editor;
  // "See " is 4 characters; position 1 is the paragraph's start.
  act(() => {
    editor.commands.setTextSelection(1 + 4 + 3);
  });
  await waitFor(() => expect(view.container.querySelector("[data-ref-title]")).toBeNull());
  expect(getMarkdown(editor)).toContain(`[local-snapshots](${HREF})`);
});

test("a labelled link keeps its label", async () => {
  const view = render(<RichTextField value={`See [the snapshots](${HREF}).`} onChange={() => {}} />);
  await settle();
  await act(async () => {
    await new Promise((r) => setTimeout(r, 20));
  });
  expect(view.container.querySelector("[data-ref-title]")).toBeNull();
});
