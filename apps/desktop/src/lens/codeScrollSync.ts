/**
 * Scrolling a `code` lens's editors together (`code.sync`): each editor is
 * anchored where its marked lines begin, and a scroll in one moves the
 * others by the same distance from their own anchors. The marked ranges
 * sit at different heights in their files, so mirroring the absolute
 * position would move the others off their marks.
 *
 * At first the marks line up at the same height on screen: two lines
 * below the top, or less when a mark is too near its file's top to scroll
 * that far (an editor can't scroll above line 1). Plain logic over an
 * editor's two setters, so it is tested without Monaco.
 */

/** What the sync moves an editor with. */
export interface Scrollable {
  setScrollTop(top: number): void;
  setScrollLeft(left: number): void;
}

interface Side {
  editor: Scrollable;
  /** Pixels from the file's top to where its mark begins; null until known. */
  markTop: number | null;
  lineHeight: number;
  /** Where it is when the marks line up; scrolls are measured from here. */
  anchor: number;
}

/** How many lines above a mark show when the marks line up. */
const PAD_LINES = 2;

export class ScrollSync {
  private sides = new Map<string, Side>();
  /** Set while the sync moves editors, so their scroll events aren't
   *  mirrored back. */
  private moving = false;

  /** Add `editor` as `key`; the returned function removes it. */
  register(key: string, editor: Scrollable): () => void {
    this.sides.set(key, { editor, markTop: null, lineHeight: 0, anchor: 0 });
    return () => {
      if (this.sides.get(key)?.editor === editor) this.sides.delete(key);
    };
  }

  /** `key`'s mark begins `markTop` pixels down its file: line the marks up. */
  place(key: string, markTop: number, lineHeight: number): void {
    const side = this.sides.get(key);
    if (!side) return;
    side.markTop = markTop;
    side.lineHeight = lineHeight;
    this.align();
  }

  /** `key` scrolled to `top`/`left`: move the others the same distance. */
  scrolled(key: string, top: number, left: number): void {
    if (this.moving) return;
    const self = this.sides.get(key);
    if (!self) return;
    const delta = top - self.anchor;
    this.move((side, other) => {
      if (side === self) return;
      other.setScrollTop(Math.max(0, side.anchor + delta));
      other.setScrollLeft(left);
    });
  }

  private align(): void {
    const placed = [...this.sides.values()].filter((s) => s.markTop !== null);
    if (placed.length === 0) return;
    const natural = PAD_LINES * Math.min(...placed.map((s) => s.lineHeight));
    const pad = Math.min(natural, ...placed.map((s) => s.markTop!));
    this.move((side, editor) => {
      if (side.markTop === null) return;
      side.anchor = Math.max(0, side.markTop - pad);
      editor.setScrollTop(side.anchor);
    });
  }

  private move(each: (side: Side, editor: Scrollable) => void): void {
    this.moving = true;
    try {
      for (const side of this.sides.values()) each(side, side.editor);
    } finally {
      this.moving = false;
    }
  }
}
