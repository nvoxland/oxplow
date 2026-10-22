import { afterEach, expect, test } from "bun:test";
import { act, cleanup, fireEvent, render } from "@testing-library/react";

import { PageNavigationContext, type PageNavigation } from "../../tabs/PageNavigationContext.js";
import { wikiPageRef } from "../../tabs/pageRefs.js";
import { Markdown } from "./Markdown.js";

// Markdown is one component with one feature set: the viewer and the
// editor link, draw code and mermaid, and hand text back the same way.

afterEach(cleanup);

const settle = () =>
  act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });

const BODY = "See [[local-snapshots]].\n\n```sh\necho hi\n```\n\n```mermaid\ngraph TD; A-->B\n```\n";

function inPage(node: React.ReactNode, opened: Array<[string, boolean]>) {
  const nav = {
    navigate: () => {},
    openRef: (ref: string, how?: { newTab?: boolean }) => opened.push([ref, how?.newTab ?? false]),
  } as unknown as PageNavigation;
  return <PageNavigationContext.Provider value={nav}>{node}</PageNavigationContext.Provider>;
}

for (const mode of ["view", "edit"] as const) {
  const props = mode === "edit" ? { onChange: () => {} } : {};

  test(`${mode}: a wikilink opens its page through the page, Cmd-click in a new tab`, async () => {
    const opened: Array<[string, boolean]> = [];
    const view = render(inPage(<Markdown value={BODY} {...props} />, opened));
    await settle();
    const href = wikiPageRef("local-snapshots").id;
    const anchor = view.container.querySelector(`a[href="${href}"]`);
    expect(anchor).not.toBeNull();
    fireEvent.click(anchor!);
    fireEvent.click(anchor!, { metaKey: true });
    expect(opened).toEqual([
      [href, false],
      [href, true],
    ]);
  });

  test(`${mode}: a code block carries a Copy of its text, a mermaid fence doesn't`, async () => {
    const copied: string[] = [];
    const clipboard = navigator.clipboard;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: async (t: string) => void copied.push(t), readText: async () => "" },
    });
    try {
      const view = render(<Markdown value={BODY} {...props} />);
      await settle();
      const buttons = view.getAllByTestId("markdown-code-copy");
      expect(buttons).toHaveLength(1);
      await act(async () => {
        fireEvent.click(buttons[0]!);
      });
      expect(copied.map((t) => t.trim())).toEqual(["echo hi"]);
    } finally {
      Object.defineProperty(navigator, "clipboard", { configurable: true, value: clipboard });
    }
  });
}

test("the editor draws a mermaid fence as a diagram block, as the viewer does", async () => {
  const view = render(<Markdown value={BODY} onChange={() => {}} />);
  await settle();
  expect(view.container.querySelector(".oxplow-rt-mermaid")).not.toBeNull();
});

test("the editor hands back text as stored: a wikilink comes out as [[…]]", async () => {
  const changes: string[] = [];
  const view = render(<Markdown value="See [[local-snapshots]]." onChange={(md) => changes.push(md)} testId="md" />);
  await settle();
  const editable = view.getByTestId("md") as HTMLElement & { editor?: { commands: { insertContentAt(pos: number, text: string): boolean } } };
  await act(async () => {
    editable.editor?.commands.insertContentAt(1, "Also ");
  });
  expect(changes.at(-1)).toBe("Also See [[local-snapshots]].");
});

test("without a change handler it is the viewer: no editable surface", async () => {
  const view = render(<Markdown value="# Hello" testId="md" />);
  await settle();
  expect(view.container.querySelector("[contenteditable]")).toBeNull();
  expect(view.getByTestId("md").querySelector("h1")?.textContent).toBe("Hello");
});
