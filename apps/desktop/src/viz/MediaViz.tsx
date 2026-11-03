import { useEffect, useState, type CSSProperties } from "react";

import { readBytesAt, statFile } from "../api.js";
import { fsRevLabel } from "../filesystem.js";
import { WORKING, type Revision } from "../revision.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import type { MediaRow } from "./vizModel.js";

/**
 * `media`: each row's file shown as what it is — an image in the page, a
 * PDF in a frame — on its filesystem at its revision, side by side, at the
 * height its place gives (a page's whole body on a viz page). The file is
 * `stat`ed first: one that isn't there, is over [`MAX_MEDIA_BYTES`] or is
 * neither an image nor a PDF says so in its strip and is never read. The
 * bytes come through `readBytesAt` and are shown from a blob URL the viz
 * gives back when it goes. See `.context/editor-and-monaco.md` "Read-only
 * code, diff and media vizes".
 */
export function MediaViz({ rows, streamId, onOpenPage, isPage = false }: { rows: MediaRow[]; streamId: string | null; onOpenPage?(ref: TabRef): void; isPage?: boolean }) {
  return (
    <div data-testid="viz-media" style={rowStyle}>
      {rows.map((r, i) => (
        <MediaFile key={`${i}:${r.filesystem ?? ""}:${r.path}@${r.revision ?? ""}`} row={r} streamId={streamId} onOpenPage={onOpenPage} divider={i > 0} isPage={isPage} />
      ))}
    </div>
  );
}

/** The largest file the viz shows: 32 MiB. */
export const MAX_MEDIA_BYTES = 32 * 1024 * 1024;

const IMAGE_TYPES: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  svg: "image/svg+xml",
  bmp: "image/bmp",
  ico: "image/x-icon",
  avif: "image/avif",
};

/** What the viz can show `path` as: an image or a PDF, by its extension;
 *  null for anything else. */
export function mediaType(path: string): { kind: "image" | "pdf"; mime: string } | null {
  const ext = /\.([a-z0-9]+)$/i.exec(path)?.[1]?.toLowerCase() ?? "";
  if (ext === "pdf") return { kind: "pdf", mime: "application/pdf" };
  const mime = IMAGE_TYPES[ext];
  return mime ? { kind: "image", mime } : null;
}

/** `bytes` as a person reads a size: `2 KB`, `1.5 MB`. */
export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  const mb = bytes / (1024 * 1024);
  return `${mb >= 10 ? Math.round(mb) : Math.round(mb * 10) / 10} MB`;
}

type Shown = { kind: "image" | "pdf"; url: string; size: number };

function MediaFile({ row, streamId, onOpenPage, divider, isPage }: { row: MediaRow; streamId: string | null; onOpenPage?(ref: TabRef): void; divider: boolean; isPage: boolean }) {
  const { path, filesystem, revision } = row;
  const where = fsRevLabel(filesystem, revision);
  const nav = useOptionalPageNavigation();
  const [shown, setShown] = useState<Shown | null>(null);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let url: string | null = null;
    setShown(null);
    setProblem(null);
    void (async () => {
      try {
        const type = mediaType(path);
        const at = { ...(filesystem ? { filesystem } : {}), ...(revision ? { rev: revision as Revision } : {}) };
        const stat = await statFile(streamId ?? "", path, at);
        if (cancelled) return;
        if (!stat) return setProblem(`Not there at ${where}`);
        if (stat.kind === "dir") return setProblem("A folder, not a file");
        if (!type) return setProblem("Not an image or a PDF");
        const size = stat.size ?? 0;
        if (size > MAX_MEDIA_BYTES) return setProblem(`Too large to show, ${size} bytes`);
        const bytes = await readBytesAt(streamId ?? "", path, at);
        if (cancelled) return;
        if (!bytes) return setProblem(`Not there at ${where}`);
        url = URL.createObjectURL(new Blob([bytes as BlobPart], { type: type.mime }));
        setShown({ kind: type.kind, url, size });
      } catch (e) {
        if (!cancelled) setProblem(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
      if (url) URL.revokeObjectURL(url);
    };
  }, [streamId, path, filesystem, revision, where]);

  const openFile = () => {
    const ref = fileRef(path, filesystem && revision ? (revision as Revision) : WORKING, filesystem);
    if (nav) nav.navigate(ref);
    else onOpenPage?.(ref);
  };

  return (
    <div data-testid="viz-media-file" style={{ ...sideStyle, borderLeft: divider ? "1px solid var(--border-subtle)" : undefined }}>
      <div style={stripStyle}>
        <span data-testid="viz-media-path" style={{ fontFamily: "var(--font-mono)", color: "var(--text-primary)" }}>
          {path}
        </span>
        <span title={`${filesystem ?? "oxplow:local"} @ ${revision ?? "current"}`} style={{ fontFamily: "var(--font-mono)" }}>
          @{where}
        </span>
        {shown ? <span>{sizeLabel(shown.size)}</span> : null}
        {problem ? (
          <span data-testid="viz-media-problem" style={{ color: "var(--severity-critical)" }}>
            {problem}
          </span>
        ) : null}
        <span style={{ flex: 1 }} />
        {isPage ? null : (
          <button type="button" className="ox-button" data-testid="viz-media-open" onClick={openFile}>
            Open file
          </button>
        )}
      </div>
      <div style={bodyStyle}>
        {shown?.kind === "image" ? <img data-testid="viz-media-image" src={shown.url} alt={path} style={imageStyle} /> : null}
        {shown?.kind === "pdf" ? <iframe data-testid="viz-media-pdf" src={shown.url} title={path} style={frameStyle} /> : null}
      </div>
    </div>
  );
}

const rowStyle: CSSProperties = { display: "flex", flex: 1, minHeight: 360, height: "100%" };
const sideStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minWidth: 0, minHeight: 0 };
const stripStyle: CSSProperties = {
  display: "flex",
  gap: "var(--space-2)",
  alignItems: "center",
  padding: "var(--space-1) var(--space-3)",
  borderBottom: "1px solid var(--border-subtle)",
  fontSize: "var(--text-xs)",
  color: "var(--text-secondary)",
};
const bodyStyle: CSSProperties = { flex: 1, minHeight: 0, display: "flex", alignItems: "center", justifyContent: "center", overflow: "auto", padding: "var(--space-3)" };
const imageStyle: CSSProperties = { maxWidth: "100%", maxHeight: "100%", objectFit: "contain" };
const frameStyle: CSSProperties = { width: "100%", height: "100%", border: 0 };
