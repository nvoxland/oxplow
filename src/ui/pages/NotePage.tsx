import type { Stream, ThreadWorkState } from "../api.js";
import { Page } from "../tabs/Page.js";
import { NoteTab } from "../components/Notes/NoteTab.js";
import type { TabRef } from "../tabs/tabState.js";
import { noteRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks } from "../tabs/useBacklinks.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";

export interface NotePageProps {
  stream: Stream | null;
  slug: string;
  threadWork: ThreadWorkState | null;
  onClosed(): void;
  onOpenNote(slug: string): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
  onOpenCommit?(sha: string): void;
  onOpenExternalUrl?(url: string): void;
}

/**
 * Thin Page wrapper around `NoteTab`. The shared chrome owns the title
 * (via `usePageTitle` inside NoteTab), the back/forward nav bar, and
 * the bookmark toggle. In-tab wikilink clicks route through the
 * navigation context so they participate in tab-level history.
 */
export function NotePage({ stream, slug, threadWork, onClosed, onOpenNote, onOpenFile, onOpenPage, onOpenCommit, onOpenExternalUrl }: NotePageProps) {
  const nav = useOptionalPageNavigation();
  const backlinkEntries = useBacklinks(noteRef(slug), stream, threadWork);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  if (!stream) {
    return (
      <Page testId="page-note" kind="note" backlinks={backlinks}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: 13 }}>
          No stream selected.
        </div>
      </Page>
    );
  }
  return (
    <Page testId="page-note" kind="note" backlinks={backlinks}>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <NoteTab
          stream={stream}
          slug={slug}
          onClosed={onClosed}
          onNavigateInternalNote={(nextSlug) => nav ? nav.navigate(noteRef(nextSlug)) : onOpenNote(nextSlug)}
          onOpenNoteInNewTab={onOpenNote}
          onOpenFile={onOpenFile}
          onOpenCommit={onOpenCommit}
          onOpenExternalUrl={onOpenExternalUrl}
        />
      </div>
    </Page>
  );
}
