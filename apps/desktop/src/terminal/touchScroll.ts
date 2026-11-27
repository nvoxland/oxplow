/** One move of a finger's drag over the terminal: the whole lines it
 *  scrolls (negative: toward older lines, as a wheel up) and the movement
 *  left over for the next move. `moved` is the finger's move down in px
 *  since the last, `rest` what was left before, `lineHeight` a row's px.
 *  Pure. */
export function touchScrollStep(rest: number, moved: number, lineHeight: number): { lines: number; rest: number } {
  if (lineHeight <= 0) return { lines: 0, rest: 0 };
  const total = rest + moved;
  const whole = Math.trunc(total / lineHeight);
  return { lines: whole === 0 ? 0 : -whole, rest: total - whole * lineHeight };
}

import { keyBytes } from "./touchKeys.js";

/** What `lines` of a wheel do in the terminal's mode, as xterm does with a
 *  wheel (which it takes only from a real one, not from a page's event): a
 *  program that took the mouse gets wheel reports at the cell (SGR, as
 *  Claude Code asks for), an alternate screen with no mouse gets the arrows
 *  (in the program's cursor-key mode), a shell scrolls its lines back. `at`
 *  is the 0-based cell. Pure. */
export function wheelAction(
  mode: { mouse: boolean; alternate: boolean; applicationCursor: boolean },
  lines: number,
  at: { col: number; row: number },
): { scroll: number } | { send: string } {
  const n = Math.abs(lines);
  if (mode.mouse) return { send: `\x1b[<${lines < 0 ? 64 : 65};${at.col + 1};${at.row + 1}M`.repeat(n) };
  if (mode.alternate) return { send: keyBytes(lines < 0 ? "up" : "down", mode.applicationCursor).repeat(n) };
  return { scroll: lines };
}

/** Let a finger scroll the terminal on `host` (xterm's mount): each move of
 *  a one-finger drag is `onLines` of whole lines (negative: toward older,
 *  as a wheel up) at the finger's point. The page does not scroll under it.
 *  Returns the undo. */
export function installTouchScroll(
  host: HTMLElement,
  lineHeight: () => number,
  onLines: (lines: number, point: { x: number; y: number }) => void,
): () => void {
  let last: number | null = null;
  let rest = 0;
  const start = (e: TouchEvent) => {
    last = e.touches.length === 1 ? e.touches[0]!.clientY : null;
    rest = 0;
  };
  const move = (e: TouchEvent) => {
    if (last === null || e.touches.length !== 1) return;
    const touch = e.touches[0]!;
    const step = touchScrollStep(rest, touch.clientY - last, lineHeight());
    last = touch.clientY;
    rest = step.rest;
    e.preventDefault();
    if (step.lines !== 0) onLines(step.lines, { x: touch.clientX, y: touch.clientY });
  };
  const end = () => {
    last = null;
    rest = 0;
  };
  host.addEventListener("touchstart", start, { passive: true });
  host.addEventListener("touchmove", move, { passive: false });
  host.addEventListener("touchend", end);
  host.addEventListener("touchcancel", end);
  return () => {
    host.removeEventListener("touchstart", start);
    host.removeEventListener("touchmove", move);
    host.removeEventListener("touchend", end);
    host.removeEventListener("touchcancel", end);
  };
}
