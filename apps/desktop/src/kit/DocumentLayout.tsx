import type { ReactNode } from "react";

import type { SqlCell } from "../api.js";
import { useExtensions } from "../extensionsStore.js";
import { LensResultView } from "../lens/LensResultView.js";
import { slotParts } from "../lens/lensModel.js";
import { useLensRuns } from "../lens/useLensRuns.js";
import { Page } from "../tabs/Page.js";
import { lensRef } from "../tabs/pageRefs.js";
import { RouteLink } from "../tabs/RouteLink.js";
import type { TabRef } from "../tabs/tabState.js";
import type { PageAction } from "./actions.js";
import { Loading } from "./Loading.js";
import { placeParts } from "./placeParts.js";
import { Section } from "./Section.js";

/** One of a document page's own sections. */
export interface DocSection {
  /** Stable: what an extension's mount names (`at: { after: <id> }`).
   *  List it with the page's slot in `SLOTS`. */
  id: string;
  title: string;
  description?: ReactNode;
  actions?: readonly PageAction[];
  testId?: string;
  body: ReactNode;
}

export interface DocumentLayoutProps {
  testId?: string;
  title: string;
  intro?: ReactNode;
  sections: readonly DocSection[];
  /** The page's parts slot (`new-session.sections`): what extensions
   *  mount there goes among `sections` by each mount's `at`. */
  slot?: string;
  /** What the slot binds (its `SLOTS` params). */
  slotParams?: Record<string, SqlCell>;
  streamId?: string | null;
  onOpenPage?(ref: TabRef): void;
  actions?: readonly PageAction[];
  rail?: ReactNode;
  railTitle?: ReactNode;
}

/** Rows a mounted section shows. */
const MOUNT_MAX_ROWS = 25;
/** A document this long gets a contents list in its rail. */
const CONTENTS_FROM = 4;

/**
 * The Document archetype: a title and intro in a reading column, then
 * sections — a heading over a rule each, no boxes — with the page's
 * controls in its rail. Extensions add sections through the page's slot,
 * placed among the page's own by their `at`.
 */
export function DocumentLayout({ testId, title, intro, sections, slot, slotParams = {}, streamId = null, onOpenPage, actions, rail, railTitle }: DocumentLayoutProps) {
  const extensions = useExtensions() ?? [];
  const mounts = slot ? slotParts(extensions, slot) : [];
  const runs = useLensRuns(mounts.map((m) => ({ key: m.id, lensId: m.lensId })), slotParams, streamId);
  type Part = { kind: "own"; section: DocSection } | { kind: "mount"; mount: (typeof mounts)[number] };
  const placed = placeParts<Part>(
    sections.map((s) => ({ id: s.id, item: { kind: "own", section: s } })),
    mounts.map((m) => ({ id: m.id, at: m.at, item: { kind: "mount", mount: m } })),
  );
  const titles = placed.map((p) => ({ id: p.id, title: p.item.kind === "own" ? p.item.section.title : p.item.mount.title }));
  const contents =
    titles.length >= CONTENTS_FROM ? (
      <nav aria-label="Contents" data-testid="document-contents" className="ox-actions" style={{ flexDirection: "column", alignItems: "flex-start" }}>
        {titles.map((t) => (
          <button
            key={t.id}
            type="button"
            className="ox-link"
            style={{ background: "none", border: "none", padding: 0, font: "inherit" }}
            onClick={() => document.querySelector(`[data-section="${CSS.escape(t.id)}"]`)?.scrollIntoView({ block: "start" })}
          >
            {t.title}
          </button>
        ))}
      </nav>
    ) : null;
  const railBody = contents || rail ? (
    <>
      {contents}
      {rail}
    </>
  ) : undefined;
  return (
    <Page testId={testId} title={title} titleInBody width="reading" actions={actions} rightRail={railBody} rightRailTitle={railTitle ?? (contents ? "Contents" : undefined)}>
      <h1 className="ox-title">{title}</h1>
      {intro ? <p className="ox-intro">{intro}</p> : null}
      {placed.map(({ id, item }) => {
        if (item.kind === "own") {
          const s = item.section;
          return (
            <Section key={id} id={s.id} title={s.title} description={s.description} actions={s.actions} testId={s.testId}>
              {s.body}
            </Section>
          );
        }
        const m = item.mount;
        const run = runs?.get(m.id);
        const to = lensRef(m.lensId);
        return (
          <Section
            key={id}
            id={id}
            source={m.extension}
            testId={`${slot}-${m.lensId}`}
            title={
              <RouteLink to={to} onNavigate={onOpenPage ? () => onOpenPage(to) : undefined} style={linkStyle}>
                {m.title}
              </RouteLink>
            }
          >
            {!run ? (
              <Loading what={m.title} />
            ) : run.error ? (
              <p className="ox-error">{run.error}</p>
            ) : run.run ? (
              <LensResultView run={run.run} onOpenPage={onOpenPage} streamId={streamId} maxRows={MOUNT_MAX_ROWS} toolbar={false} />
            ) : null}
          </Section>
        );
      })}
    </Page>
  );
}

const linkStyle = {
  background: "none",
  border: "none",
  padding: 0,
  font: "inherit",
  color: "inherit",
  cursor: "pointer",
  textAlign: "left",
} as const;
