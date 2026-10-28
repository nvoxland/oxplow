/**
 * A page's Contents, in its rail: the markdown headings (`h2`, `h3`) in
 * the page's body and, once there are four or more, its sections'
 * headings, in order, the one on screen marked; a click scrolls to it.
 * Nothing is declared: a page whose body has markdown with headings (a
 * viewer or an editor, `.oxplow-md`) or a document of several sections
 * gets it, one without shows nothing. It follows the body as it changes
 * (an edit, a re-run).
 */
import { useEffect, useState } from "react";

import { RailGroup } from "../kit/RailGroup.js";

interface Heading {
  el: HTMLElement;
  text: string;
  level: 2 | 3;
}

const HEADINGS = ".oxplow-md h2, .oxplow-md h3, h2.ox-section-title";
const SECTION_HEADING = "ox-section-title";
/** Sections a page has before Contents lists them. */
export const CONTENTS_FROM = 4;

/** The element that scrolls `el`: itself or its nearest ancestor that does. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let at: HTMLElement | null = el; at; at = at.parentElement) {
    const overflow = getComputedStyle(at).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && at.scrollHeight > at.clientHeight) return at;
  }
  return null;
}

/** A section that drew nothing under its heading (not one folded shut):
 *  hidden (the kit's `.ox-section` rule), so not a place to go to. */
function emptySection(el: HTMLElement): boolean {
  const section = el.closest<HTMLElement>(".ox-section");
  return (
    !!section &&
    !section.hasAttribute("data-collapsed") &&
    Array.from(section.children).every((c) => c.classList.contains("ox-section-head"))
  );
}

function headingsIn(root: HTMLElement): Heading[] {
  const all = Array.from(root.querySelectorAll<HTMLElement>(HEADINGS)).filter((el) => !emptySection(el));
  const sections = all.filter((el) => el.classList.contains(SECTION_HEADING)).length;
  return all
    .filter((el) => sections >= CONTENTS_FROM || !el.classList.contains(SECTION_HEADING))
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
    <RailGroup title="Contents" testId="viz-rail-contents">
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
