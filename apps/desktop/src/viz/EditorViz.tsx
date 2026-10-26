/// `editor` viz (`.context/editor-and-monaco.md` "The editor viz"): the
/// working tree's file its first row names, edited at full size. Its
/// buffer is the window's (`editor/openFiles.ts`), shared with every tab
/// showing the file; it tells its tab when it holds unsaved edits and how
/// to save and find (`useTabEditing`: the ●, File → Save, the tab cap).
/// Saving runs `oxplow.file.save`. The disk changing under unsaved edits
/// asks: Reload from disk, or keep them.
import { useEffect, useRef, type CSSProperties } from "react";

import { EditorPane } from "../components/EditorPane.js";
import { isDirty, useBuffer } from "../editor/buffers.js";
import { buffers } from "../editor/openFiles.js";
import { recordOpError } from "../opErrors.js";
import { WORKING } from "../revision.js";
import { useOptionalPageNavigation, useTabEditing } from "../tabs/PageNavigationContext.js";
import { diffRef, fileRefAt, gitCommitRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { useVizStream } from "./VizStream.js";

export function EditorViz({ row, onOpenPage }: { row: { path: string; line: number | null } | null; onOpenPage?(ref: TabRef): void }) {
  const stream = useVizStream();
  const nav = useOptionalPageNavigation();
  const path = row?.path ?? null;
  useEffect(() => {
    if (stream && path) void buffers.open(stream, path);
  }, [stream, path]);
  const buffer = useBuffer(buffers, stream?.id ?? null, path);
  const findRef = useRef<(() => void) | null>(null);
  const save = () => {
    if (!stream || !path) return;
    buffers.save(stream.id, path).catch((e: unknown) =>
      recordOpError({ label: "Save", message: e instanceof Error ? e.message : String(e) }),
    );
  };
  useTabEditing({ dirty: isDirty(buffer), save, find: () => findRef.current?.() });
  // Another page opens in its own tab: the file stays where it is.
  const open = (ref: TabRef, state?: Record<string, unknown>) => {
    if (nav) nav.navigate(ref, { newTab: true, ...(state ? { state } : {}) });
    else onOpenPage?.(ref);
  };

  if (!stream) return <p className="ox-empty-line">No project is open.</p>;
  if (!path) return <p className="ox-empty-line">This page names no file.</p>;
  if (!buffer || buffer.state === "loading") return <p className="ox-empty-line" data-testid="viz-editor-loading">Opening {path}…</p>;
  if (buffer.state === "missing") {
    return (
      <p className="ox-empty-line" data-testid="viz-editor-missing">
        <code>{path}</code> isn't a file in this stream's working tree.
      </p>
    );
  }
  if (buffer.state === "error") return <p className="ox-error">{buffer.error}</p>;
  return (
    <div data-testid="viz-editor" style={fillStyle}>
      <EditorPane
        stream={stream}
        filePath={path}
        line={row?.line ?? null}
        onSave={save}
        findRef={findRef}
        onNavigateToLocation={(target) => open(fileRefAt(target.path, target.line))}
        onRevealCommit={(sha) => open(gitCommitRef(sha))}
        onCompareWithClipboard={(selection, file) => {
          void navigator.clipboard.readText().then(
            (clipboard) => {
              // The two texts are no address: they ride as the new entry's
              // history state (`DiffViz` reads them); WORKING on both sides
              // is the sentinel they replace. Each compare is its own tab.
              const ref = diffRef({
                path: file,
                leftVersion: WORKING,
                rightVersion: WORKING,
                baseLabel: "clipboard",
                labelOverride: `selection vs clipboard (${Date.now()})`,
              });
              open(ref, { diffText: { left: selection, right: clipboard } });
            },
            (e: unknown) => recordOpError({ label: "Compare with Clipboard", message: `Clipboard read failed: ${String(e)}` }),
          );
        }}
      />
      {buffer.conflict !== null ? <FileChangedDialog path={path} onReload={() => buffers.reload(stream.id, path)} onKeepMine={() => buffers.keepMine(stream.id, path)} /> : null}
    </div>
  );
}

/** The disk changed under unsaved edits: take the disk's text, or keep the
 *  draft and count the disk's as the saved one. */
function FileChangedDialog({ path, onReload, onKeepMine }: { path: string; onReload(): void; onKeepMine(): void }) {
  return (
    <div data-testid="viz-editor-conflict" style={overlayStyle}>
      <div style={dialogStyle} role="dialog" aria-label="File changed on disk">
        <div>
          <div style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>File changed on disk</div>
          <div style={{ color: "var(--muted)", lineHeight: 1.5 }}>
            <code>{path}</code> changed on disk while you had unsaved edits. Reload the file from disk or keep your
            draft and treat the new disk content as the latest saved version.
          </div>
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
          <button type="button" data-testid="viz-editor-keep-mine" onClick={onKeepMine} style={secondaryStyle}>
            Keep my changes
          </button>
          <button type="button" data-testid="viz-editor-reload" onClick={onReload} style={primaryStyle}>
            Reload from disk
          </button>
        </div>
      </div>
    </div>
  );
}

const fillStyle: CSSProperties = { display: "flex", flexDirection: "column", flex: 1, minHeight: 0 };
const overlayStyle: CSSProperties = {
  position: "fixed",
  inset: 0,
  background: "rgba(0, 0, 0, 0.65)",
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  zIndex: 1000,
  padding: 24,
};
const dialogStyle: CSSProperties = {
  width: "min(520px, 100%)",
  background: "var(--bg-2)",
  border: "1px solid var(--border)",
  borderRadius: 8,
  padding: 20,
  boxShadow: "0 0 0 1px rgba(255,255,255,0.12), 0 12px 40px rgba(0, 0, 0, 0.4)",
  display: "flex",
  flexDirection: "column",
  gap: 16,
};
const secondaryStyle: CSSProperties = {
  background: "transparent",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  padding: "8px 14px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
};
const primaryStyle: CSSProperties = {
  background: "var(--accent)",
  color: "#fff",
  border: "none",
  padding: "8px 14px",
  borderRadius: 4,
  cursor: "pointer",
  fontFamily: "inherit",
};
