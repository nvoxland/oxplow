import { afterEach, expect, test } from "bun:test";
import { act, cleanup, render } from "@testing-library/react";

import { RichTextField } from "./RichTextField.js";

afterEach(cleanup);

const MERMAID = "Intro\n\n```mermaid\ngraph TD\n  A --> B\n```";

// Wiki pages mount the field before the body loads, then hand it the real
// value. That upstream sync creates MermaidBlock's React NodeView, which
// Tiptap 3 renders with flushSync once the editor content is mounted — so the
// sync must not run inside React's commit phase, or React logs "flushSync was
// called from inside a lifecycle method".
test("syncing an upstream value with a mermaid block doesn't flushSync mid-commit", async () => {
  const errors: string[] = [];
  const original = console.error;
  console.error = (...args: unknown[]) => {
    errors.push(args.map(String).join(" "));
  };
  try {
    const { container, rerender } = render(<RichTextField value="" onCommit={() => {}} />);
    await act(async () => {
      await new Promise((r) => setTimeout(r, 0));
    });
    await act(async () => {
      rerender(<RichTextField value={MERMAID} onCommit={() => {}} />);
      await new Promise((r) => setTimeout(r, 0));
    });
    expect(container.querySelector(".ProseMirror")?.textContent).toContain("Intro");
  } finally {
    console.error = original;
  }
  expect(errors.filter((e) => e.includes("flushSync"))).toEqual([]);
});
