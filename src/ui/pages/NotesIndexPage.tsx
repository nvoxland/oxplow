import type { Stream } from "../api.js";
import { Page } from "../tabs/Page.js";
import { NotesPane } from "../components/Notes/NotesPane.js";

export interface NotesIndexPageProps {
  stream: Stream | null;
  selectedSlug: string | null;
  onOpenNote: (slug: string) => void;
}

/**
 * Thin Page wrapper around the existing NotesPane (wiki notes index).
 */
export function NotesIndexPage({ stream, selectedSlug, onOpenNote }: NotesIndexPageProps) {
  return (
    <Page testId="page-notes-index" title="Notes" kind="wiki">
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <NotesPane stream={stream} selectedSlug={selectedSlug} onOpenNote={onOpenNote} />
      </div>
    </Page>
  );
}
