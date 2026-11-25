/// A component's `reveal(top, bottom)` (bridge protocol 14): a frame as tall
/// as what it draws scrolls with its page, which its sandboxed frame can't
/// scroll; the host brings that part of it into view, as `scrollIntoView`
/// with `block: "nearest"` would.

/** Scroll `frame`'s nearest scrolling ancestor so `[top, bottom]` (px from
 *  the frame's top) is in view, moving as little as it can: a part below
 *  shows its bottom at the view's bottom, one above its top at the top, and
 *  one in view doesn't move. Nothing scrolls when no ancestor does. */
export function revealInFrame(frame: HTMLElement, top: number, bottom: number): void {
  const scroller = scrollingAncestor(frame);
  if (!scroller) return;
  const view = scroller.getBoundingClientRect();
  const at = frame.getBoundingClientRect().top;
  const from = at + top;
  const to = at + bottom;
  if (from < view.top) scroller.scrollTop -= view.top - from;
  else if (to > view.bottom) scroller.scrollTop += Math.min(to - view.bottom, from - view.top);
}

function scrollingAncestor(el: HTMLElement): HTMLElement | null {
  for (let at = el.parentElement; at; at = at.parentElement) {
    const overflow = getComputedStyle(at).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && at.scrollHeight > at.clientHeight) return at;
  }
  return null;
}
