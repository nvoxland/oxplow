import { describe, expect, it } from "bun:test";
import type { Node as PMNode } from "@tiptap/pm/model";

import { BLOCK_SEP, flatten } from "./CommentDecorations.js";

/// Minimal fake doc: `descendants` replays a pre-built node list in
/// document order. flatten only reads isBlock/isText/text + the pos.
function fakeDoc(nodes: { isBlock?: boolean; isText?: boolean; text?: string; pos: number }[]): PMNode {
  return {
    descendants(cb: (node: unknown, pos: number) => boolean | void) {
      for (const n of nodes) cb({ isBlock: !!n.isBlock, isText: !!n.isText, text: n.text }, n.pos);
    },
  } as unknown as PMNode;
}

describe("flatten", () => {
  it("joins two blocks with a separator and keeps map 1:1 with text", () => {
    const { text, map } = flatten(
      fakeDoc([
        { isBlock: true, pos: 0 },
        { isText: true, text: "ab", pos: 1 },
        { isBlock: true, pos: 4 },
        { isText: true, text: "cd", pos: 5 },
      ]),
    );
    expect(text).toBe(`ab${BLOCK_SEP}cd`);
    expect(map.length).toBe(text.length);
    // a,b -> doc 1,2 ; separator -> the 2nd block's pos (4) ; c,d -> 5,6
    expect(map).toEqual([1, 2, 4, 5, 6]);
  });

  it("emits no leading separator and no separator between text in one block", () => {
    const { text, map } = flatten(
      fakeDoc([
        { isBlock: true, pos: 0 },
        { isText: true, text: "foo", pos: 1 },
        { isText: true, text: "bar", pos: 4 }, // same block, adjacent text node
      ]),
    );
    expect(text).toBe("foobar");
    expect(map).toEqual([1, 2, 3, 4, 5, 6]);
  });
});
