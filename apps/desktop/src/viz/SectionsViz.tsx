/**
 * `viz: sections`: one subsection per row, in row order — a heading
 * (linked when the row names a page), the row's `columns` as a short facts
 * list, its markdown body. A thing gets a section, not a table row
 * (`.context/usability.md` "Pages are informational").
 *
 * Each subsection is a real section of the page: `kit/Section.tsx`'s chrome,
 * its heading one level below the section the viz is in (so Contents
 * lists it under its parent), a `data-node` of `<viz node>/<row key>` that
 * stays the same as the rows change, and the viz's row actions in the
 * row's right-click menu (and its primary action on click), as for a list.
 */
import type { HTMLAttributes, ReactNode } from "react";

import { Markdown } from "../components/Markdown/Markdown.js";
import { Facts, type Fact } from "../kit/Facts.js";
import { Section } from "../kit/Section.js";
import { SUBSECTIONS_ATTR } from "./Contents.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { SectionRow } from "./vizModel.js";

export interface SectionsVizProps {
  /** The viz node's id (the page's node), the prefix of its subsections'. */
  node: string;
  sections: readonly SectionRow[];
  /** A row's facts: its displayed `columns`, drawn. */
  facts(row: SqlCell[]): Fact[];
  /** A row's right-click menu (its actions) and keys. */
  rowMenu(row: SqlCell[]): HTMLAttributes<HTMLElement>;
  /** The heading, as a link when the row names a page. */
  heading(section: SectionRow): ReactNode;
}

export function SectionsViz({ node, sections, facts, rowMenu, heading }: SectionsVizProps) {
  return (
    <div data-testid="viz-sections" {...{ [SUBSECTIONS_ATTR]: sections.length }}>
      {sections.map((s) => {
        const rows = facts(s.row);
        const id = `${node}/${s.key}`;
        return (
          <div key={s.key} data-node={id} data-testid={`node-${id}`} {...rowMenu(s.row)}>
            <Section id={id} title={heading(s)} keepEmpty testId={`viz-section-${s.key}`}>
              {rows.length > 0 ? <Facts testId={`viz-section-facts-${s.key}`} facts={rows} /> : null}
              {s.body ? <Markdown value={s.body} testId={`viz-section-body-${s.key}`} /> : null}
            </Section>
          </div>
        );
      })}
    </div>
  );
}
