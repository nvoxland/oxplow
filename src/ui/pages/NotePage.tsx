import type { Stream, ThreadWorkState } from "../api.js";
import { Page } from "../tabs/Page.js";
import { NoteTab } from "../components/Notes/NoteTab.js";
import type { TabRef } from "../tabs/tabState.js";
import { noteRef } from "../tabs/pageRefs.js";
import { BacklinksList } from "../tabs/BacklinksList.js";
import { useBacklinks } from "../tabs/useBacklinks.js";

export interface NotePageProps {
  stream: Stream | null;
  slug: string;
  threadWork: ThreadWorkState | null;
  onClosed(): void;
  onOpenNote(slug: string): void;
  onOpenFile(path: string): void;
  onOpenPage(ref: TabRef): void;
  /** Optional handler for git-commit wikilink clicks — routes to GitCommitPage. */
  onOpenCommit?(sha: string): void;
  /** Optional handler for external URL clicks — routes to in-app tab. */
  onOpenExternalUrl?(url: string): void;
}

/**
 * Thin Page wrapper around `NoteTab`. Phase 4 routes `note:` refs
 * through the page chrome so a Backlinks panel sits at the bottom of
 * every wiki note alongside the existing in-tab freshness UI.
 */
export function NotePage({ stream, slug, threadWork, onClosed, onOpenNote, onOpenFile, onOpenPage, onOpenCommit, onOpenExternalUrl }: NotePageProps) {
  const backlinkEntries = useBacklinks(noteRef(slug), stream, threadWork);
  const backlinks = {
    count: backlinkEntries.length,
    body: <BacklinksList entries={backlinkEntries} onOpenPage={onOpenPage} />,
  };
  if (!stream) {
    return (
      <Page testId="page-note" title={slug} kind="note" backlinks={backlinks}>
        <div style={{ padding: "16px 20px", color: "var(--text-secondary)", fontSize: 13 }}>
          No stream selected.
        </div>
      </Page>
    );
  }
  return (
    <Page testId="page-note" title={slug} kind="note" backlinks={backlinks}>
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <NoteTab
          stream={stream}
          slug={slug}
          onClosed={onClosed}
          onOpenNoteInNewTab={onOpenNote}
          onOpenFile={onOpenFile}
          onOpenCommit={onOpenCommit}
          onOpenExternalUrl={onOpenExternalUrl}
        />
      </div>
    </Page>
  );
}
