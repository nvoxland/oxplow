/**
 * `viz: sections`: one subsection per row, in row order — a heading
 * (an arrow after it opens the row's page, when it names one; with
 * `sections.group`, rows with the same group are under one group section, a
 * level above them), the row's `columns` as a short facts
 * list, its markdown body. A thing gets a section, not a table row
 * (`.context/usability.md` "Pages are informational").
 *
 * Each subsection is a real section of the page: `kit/Section.tsx`'s chrome,
 * its heading one level below the section the viz is in (so Contents
 * lists it under its parent), a `data-node` of `<viz node>/<row key>` that
 * stays the same as the rows change, and the viz's row actions in the
 * row's right-click menu (and its primary action on click), as for a list.
 */
import type { HTMLAttributes } from "react";

import { Markdown } from "../components/Markdown/Markdown.js";
import { Facts, type Fact } from "../kit/Facts.js";
import { Section } from "../kit/Section.js";
import { SUBSECTIONS_ATTR } from "./Contents.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";
import type { TabRef } from "../tabs/tabState.js";
import { sectionParts, type LinkTarget, type SectionRow } from "./vizModel.js";

export interface SectionsVizProps {
  /** The viz node's id (the page's node), the prefix of its subsections'. */
  node: string;
  sections: readonly SectionRow[];
  /** A row's facts: its displayed `columns`, drawn. */
  facts(row: SqlCell[]): Fact[];
  /** A row's right-click menu (its actions) and keys. */
  rowMenu(row: SqlCell[]): HTMLAttributes<HTMLElement>;
  /** What opens a heading's page, when the row names one. */
  navigate(link: LinkTarget): ((ref: TabRef, opts?: { newTab?: boolean }) => void) | undefined;
}

export function SectionsViz({ node, sections, facts, rowMenu, navigate }: SectionsVizProps) {
  const parts = sectionParts(sections);
  const opens = (title: string, link: LinkTarget | null) => (link ? { to: link, name: title, onNavigate: navigate(link) } : undefined);
  const subsection = (s: SectionRow) => {
    const rows = facts(s.row);
    const id = `${node}/${s.key}`;
    return (
      <div key={s.key} data-node={id} data-testid={`node-${id}`} {...rowMenu(s.row)}>
        <Section id={id} title={s.title} opens={opens(s.title, s.link)} keepEmpty testId={`viz-section-${s.key}`}>
          {rows.length > 0 ? <Facts testId={`viz-section-facts-${s.key}`} facts={rows} /> : null}
          {s.body ? <Markdown value={s.body} testId={`viz-section-body-${s.key}`} /> : null}
        </Section>
      </div>
    );
  };
  return (
    <div data-testid="viz-sections" {...{ [SUBSECTIONS_ATTR]: parts.length }}>
      {parts.map((part) =>
        part.kind === "row" ? (
          subsection(part.section)
        ) : (
          // A group is a section of its own, its rows' subsections one level in;
          // it carries its row count for Contents' cap, as the viz does.
          <Section key={`group:${part.name}`} id={`${node}/group:${part.name}`} title={part.name} opens={opens(part.name, part.link)} keepEmpty testId={`viz-group-${part.name}`}>
            <div {...{ [SUBSECTIONS_ATTR]: part.rows.length }}>{part.rows.map(subsection)}</div>
          </Section>
        ),
      )}
    </div>
  );
}
