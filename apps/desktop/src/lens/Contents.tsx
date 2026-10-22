/**
 * A page's Contents, in its rail: the markdown headings (`h2`, `h3`) in
 * the page's body, in order, the one on screen marked; a click scrolls
 * to it. Nothing is declared: a page whose body has markdown with
 * headings (a viewer or an editor, `.oxplow-md`) gets it, one without
 * shows nothing. It follows the body as it changes (an edit, a re-run).
 */
import { useEffect, useState } from "react";

import { RailGroup } from "../kit/RailGroup.js";

interface Heading {
  el: HTMLElement;
  text: string;
  level: 2 | 3;
}

const HEADINGS = ".oxplow-md h2, .oxplow-md h3";

/** The element that scrolls `el`: itself or its nearest ancestor that does. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let at: HTMLElement | null = el; at; at = at.parentElement) {
    const overflow = getComputedStyle(at).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && at.scrollHeight > at.clientHeight) return at;
  }
  return null;
}

function headingsIn(root: HTMLElement): Heading[] {
  return Array.from(root.querySelectorAll<HTMLElement>(HEADINGS))
    .map((el): Heading => ({ el, text: el.textContent?.trim() ?? "", level: el.tagName === "H2" ? 2 : 3 }))
    .filter((h) => h.text.length > 0);
}

/** `root` is the page's body. */
export function Contents({ root }: { root: HTMLElement | null }) {
  const [headings, setHeadings] = useState<Heading[]>([]);
  const [active, setActive] = useState(0);

  // The headings, read again (once a frame at most) as the body changes.
  useEffect(() => {
    if (!root) {
      setHeadings([]);
      return;
    }
    let frame: number | null = null;
    const read = () => {
      frame = null;
      const next = headingsIn(root);
      setHeadings((prev) => (prev.length === next.length && prev.every((h, i) => h.el === next[i]!.el && h.text === next[i]!.text) ? prev : next));
    };
    read();
    const observer = new MutationObserver(() => {
      if (frame === null) frame = requestAnimationFrame(read);
    });
    observer.observe(root, { childList: true, subtree: true, characterData: true });
    return () => {
      observer.disconnect();
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [root]);

  // The heading on screen: the last whose top has passed the scroller's.
  useEffect(() => {
    if (!root || headings.length === 0) return;
    const scroller = scrollParent(root);
    const compute = () => {
      const top = scroller ? scroller.getBoundingClientRect().top : 0;
      let current = 0;
      headings.forEach((h, i) => {
        if (h.el.getBoundingClientRect().top - top - 8 <= 0) current = i;
      });
      setActive(current);
    };
    compute();
    const target: HTMLElement | Window = scroller ?? window;
    target.addEventListener("scroll", compute, { passive: true });
    return () => target.removeEventListener("scroll", compute);
  }, [root, headings]);

  if (headings.length === 0) return null;
  return (
    <RailGroup title="Contents" testId="lens-rail-contents">
      <nav aria-label="Contents" className="ox-contents">
        {headings.map((h, i) => (
          <button
            key={i}
            type="button"
            className="ox-contents-item"
            data-level={h.level}
            data-active={i === active ? "true" : undefined}
            title={h.text}
            onClick={() => h.el.scrollIntoView({ block: "start", behavior: "smooth" })}
          >
            {h.text}
          </button>
        ))}
      </nav>
    </RailGroup>
  );
}
