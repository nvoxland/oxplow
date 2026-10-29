/**
 * A page's Contents, in its rail: the markdown headings (`h2`, `h3`) in
 * the page's body and, once the page has two or more sections (a
 * `sections` viz's subsections count), its sections' headings, in order,
 * the one on screen marked; a click scrolls to it. A section nested in
 * another (a `sections` viz's subsections, and its groups) is listed under
 * its parent, the first `CONTENTS_SUBSECTIONS_MAX` of them per parent, then
 * one "… N more" that scrolls to the parent.
 * Nothing is declared: a page whose body has markdown with headings (a
 * viewer or an editor, `.oxplow-md`) or a document of several sections
 * gets it, one without shows nothing. It follows the body as it changes
 * (an edit, a re-run).
 */
import { useEffect, useState } from "react";

import { RailGroup } from "../kit/RailGroup.js";

interface Heading {
  /** What a click scrolls to. */
  el: HTMLElement;
  /** Where it sits in the page, when that isn't `el` (a "… N more" is
   *  where the first left-out subsection is, but goes to the parent). */
  at?: HTMLElement;
  text: string;
  level: 2 | 3 | 4;
}

const HEADINGS = ".oxplow-md h2, .oxplow-md h3, .ox-section-title";
const SECTION_HEADING = "ox-section-title";
/** Sections a page has before Contents lists them: a page with a single
 *  one has none. `railHas` counts with it. */
export const CONTENTS_FROM = 2;
/** The most subsections of one `sections` viz Contents lists, under their
 *  parent; a viz with more (every page of an extension, a hundred of
 *  them) ends in one "… N more" that goes to the parent. */
export const CONTENTS_SUBSECTIONS_MAX = 10;
/** What a `sections` viz's root carries: its row count. */
export const SUBSECTIONS_ATTR = "data-sections";

/** The element that scrolls `el`: itself or its nearest ancestor that does. */
function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let at: HTMLElement | null = el; at; at = at.parentElement) {
    const overflow = getComputedStyle(at).overflowY;
    if ((overflow === "auto" || overflow === "scroll") && at.scrollHeight > at.clientHeight) return at;
  }
  return null;
}

/** A section that drew nothing under its heading (not one folded shut):
 *  hidden (the kit's `.ox-section` rule), so not a place to go to. A
 *  section that `keepEmpty` is drawn, so it stays. */
function emptySection(el: HTMLElement): boolean {
  const section = el.closest<HTMLElement>(".ox-section");
  return (
    !!section &&
    !section.hasAttribute("data-keep") &&
    !section.hasAttribute("data-collapsed") &&
    Array.from(section.children).every((c) => c.classList.contains("ox-section-head"))
  );
}

function headingsIn(root: HTMLElement): Heading[] {
  const all = Array.from(root.querySelectorAll<HTMLElement>(HEADINGS)).filter((el) => !emptySection(el));
  const sections = all.filter((el) => el.classList.contains(SECTION_HEADING)).length;
  const out: Heading[] = [];
  // How many each `sections` viz's container has listed so far, and the
  // sections left out (what is under one is left out with it).
  const listed = new Map<HTMLElement, number>();
  const leftOut = new Set<Element>();
  for (const el of all) {
    const isSection = el.classList.contains(SECTION_HEADING);
    if (isSection && sections < CONTENTS_FROM) continue;
    const own = el.closest("section");
    if (isSection && own) {
      let under = false;
      for (let up = own.parentElement?.closest("section"); up; up = up.parentElement?.closest("section")) {
        if (leftOut.has(up)) under = true;
      }
      if (under) {
        leftOut.add(own);
        continue;
      }
    }
    const group = isSection ? el.closest<HTMLElement>(`[${SUBSECTIONS_ATTR}]`) : null;
    if (group) {
      const n = (listed.get(group) ?? 0) + 1;
      listed.set(group, n);
      if (n > CONTENTS_SUBSECTIONS_MAX) {
        if (own) leftOut.add(own);
        // One entry for the rest, at the first one left out.
        if (n === CONTENTS_SUBSECTIONS_MAX + 1) {
          const more = Number(group.getAttribute(SUBSECTIONS_ATTR)) - CONTENTS_SUBSECTIONS_MAX;
          const parent = group.closest("section")?.querySelector<HTMLElement>(`.${SECTION_HEADING}`) ?? group;
          out.push({ el: parent, at: el, text: `… ${more} more`, level: levelOf(el) });
        }
        continue;
      }
    }
    const text = el.textContent?.trim() ?? "";
    if (text.length > 0) out.push({ el, text, level: levelOf(el) });
  }
  return out;
}

/** A heading's indent: its `h` level, from `h2` (the page's top) to `h4`. */
function levelOf(el: HTMLElement): 2 | 3 | 4 {
  const n = Number(el.tagName.slice(1));
  return n <= 2 ? 2 : n === 3 ? 3 : 4;
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
        if ((h.at ?? h.el).getBoundingClientRect().top - top - 8 <= 0) current = i;
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
