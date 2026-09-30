import React, { useEffect, useRef, useState } from "react";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { readAt, type Stream } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { shortRevisionLabel, type Revision } from "../revision.js";
import type { DuplicateBlockPayload } from "../tabs/pageRefs.js";

const HIGHLIGHT_STYLE_ID = "oxplow-duplicate-block-style";

function ensureHighlightStyle(): void {
  if (typeof document === "undefined") return;
  if (document.getElementById(HIGHLIGHT_STYLE_ID)) return;
  const style = document.createElement("style");
  style.id = HIGHLIGHT_STYLE_ID;
  style.textContent = `
.monaco-editor .oxplow-duplicate-block-line {
  background: rgba(255, 200, 80, 0.18) !important;
}
.monaco-editor .oxplow-duplicate-block-margin {
  background: rgba(255, 200, 80, 0.35) !important;
}
`;
  document.head.appendChild(style);
}

export interface DuplicateBlockPageProps {
  stream: Stream;
  payload: DuplicateBlockPayload;
  visible: boolean;
  onJumpToSource(path: string, version: Revision): void;
}

/**
 * Side-by-side viewer for a single duplicate-block finding. Renders
 * two read-only Monaco editors stacked horizontally, each scrolled so
 * its duplicated range starts at the top of the viewport — making the
 * two ranges line up visually. The duplicated lines on each side are
 * highlighted with a soft accent background.
 *
 * Not a Monaco diff editor: a real text-diff would re-align line-by-
 * line on content, which is exactly what we don't want — duplication
 * findings can come from completely different files where Monaco's
 * diff would line up unrelated lines. We want literal range-aligned
 * side-by-side.
 */
/**
 * Shared scroll-sync bus. Mirrors scroll DELTA (not absolute
 * scrollTop) between the two sides — the duplicate ranges live at
 * different absolute Y offsets in their files, so absolute mirroring
 * would jerk the peer to a position that doesn't correspond to its
 * own duplicate range. Each side records the scrollTop at initial
 * alignment as its anchor; subsequent scrolls propagate the delta
 * from anchor to anchor. The `lock` flag breaks the otherwise
 * infinite ping-pong.
 */
interface ScrollSyncSide {
  editor: any;
  /** Pixel offset of this side's highlight start from line 1.
   *  Set once at align time; used by `recomputeAlignment` to pick
   *  a shared pad that respects both sides' headroom. */
  topForStart: number;
  /** Line height in pixels — used by recomputeAlignment to bound
   *  the pad. Both sides should report the same value (same
   *  theme/font), but we read it per-side and use the minimum. */
  lineHeight: number;
  /** Scroll position the side is "at" when the highlight is
   *  first revealed; subsequent user scrolls compute delta from
   *  this. Recomputed whenever the shared pad changes. */
  anchor: number;
}
interface ScrollSyncBus {
  left: ScrollSyncSide | null;
  right: ScrollSyncSide | null;
  lock: boolean;
}

const PAD_LINES = 2;

/**
 * Pick a shared pad both sides can satisfy and apply it.
 *
 * The "natural" pad is `PAD_LINES * lineHeight` (highlight starts a
 * couple of lines below the viewport top). But if either side's
 * highlight is too close to the top of its file (e.g. line 1), the
 * editor can't scroll to a negative offset — the best it can do is
 * pin the highlight at the very top. To keep the two highlights at
 * the SAME screen Y we use the smaller of (left.top, right.top,
 * natural_pad) as the effective pad on both sides.
 */
function recomputeAlignment(bus: ScrollSyncBus): void {
  const { left, right } = bus;
  if (!left && !right) return;
  const natural = PAD_LINES * Math.min(
    left?.lineHeight ?? Infinity,
    right?.lineHeight ?? Infinity,
  );
  const cap = Math.min(
    left?.topForStart ?? Infinity,
    right?.topForStart ?? Infinity,
  );
  const pad = Math.min(natural, cap);
  bus.lock = true;
  try {
    if (left) {
      left.anchor = Math.max(0, left.topForStart - pad);
      left.editor.setScrollTop(left.anchor, 1 /* Immediate */);
    }
    if (right) {
      right.anchor = Math.max(0, right.topForStart - pad);
      right.editor.setScrollTop(right.anchor, 1);
    }
  } finally {
    bus.lock = false;
  }
}

export function DuplicateBlockPage({ stream, payload, visible, onJumpToSource }: DuplicateBlockPageProps) {
  const leftBase = payload.leftPath.split("/").pop() ?? payload.leftPath;
  const rightBase = payload.rightPath.split("/").pop() ?? payload.rightPath;
  usePageTitle(`${leftBase} ↔ ${rightBase} (duplicate)`);

  void visible;

  const syncRef = useRef<ScrollSyncBus>({ left: null, right: null, lock: false });

  return (
    <Page testId="page-duplicate-block" kind="duplicate-block">
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
        <div
          style={{
            padding: "4px 10px",
            borderBottom: "1px solid var(--border-subtle)",
            color: "var(--text-muted)",
            fontSize: 11,
            display: "flex",
            gap: 16,
            alignItems: "center",
          }}
        >
          <span>
            Duplicated block:{" "}
            <strong style={{ color: "var(--text-primary)" }}>
              {payload.leftEnd - payload.leftStart + 1} lines
            </strong>
          </span>
          {(() => {
            const leftLabel = shortRevisionLabel(payload.leftVersion);
            const rightLabel = shortRevisionLabel(payload.rightVersion);
            const combined = leftLabel === rightLabel
              ? `Both at @${leftLabel}`
              : `Left @${leftLabel}, right @${rightLabel}`;
            return (
              <span style={{ color: "var(--text-muted)" }}>{combined}</span>
            );
          })()}
        </div>
        <div style={{ display: "flex", flex: 1, minHeight: 0 }}>
          <DuplicateSide
            stream={stream}
            path={payload.leftPath}
            version={payload.leftVersion}
            startLine={payload.leftStart}
            endLine={payload.leftEnd}
            onJumpToSource={onJumpToSource}
            side="left"
            syncRef={syncRef}
          />
          <div style={{ width: 1, background: "var(--border-subtle)" }} />
          <DuplicateSide
            stream={stream}
            path={payload.rightPath}
            version={payload.rightVersion}
            startLine={payload.rightStart}
            endLine={payload.rightEnd}
            onJumpToSource={onJumpToSource}
            side="right"
            syncRef={syncRef}
          />
        </div>
      </div>
    </Page>
  );
}

interface SideProps {
  stream: Stream;
  path: string;
  version: Revision;
  startLine: number;
  endLine: number;
  onJumpToSource(path: string, version: Revision): void;
  side: "left" | "right";
  syncRef: React.MutableRefObject<ScrollSyncBus>;
}

function DuplicateSide({
  stream,
  path,
  version,
  startLine,
  endLine,
  onJumpToSource,
  side,
  syncRef,
}: SideProps) {
  const versionLabel = shortRevisionLabel(version);
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const [editorReady, setEditorReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    ensureHighlightStyle();
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      monacoRef.current = monaco;
      const editor = monaco.editor.create(hostRef.current, {
        automaticLayout: true,
        readOnly: true,
        theme: "vs-dark",
        minimap: { enabled: false },
        renderLineHighlight: "none",
        scrollBeyondLastLine: false,
      });
      editorRef.current = editor;
      syncRef.current[side] = { editor, topForStart: 0, lineHeight: 18, anchor: 0 };
      // Mirror scroll DELTA from this side's anchor to the peer's
      // anchor. The duplicate ranges live at different absolute Y
      // in their respective files, so naive absolute mirroring
      // would jump the peer off its highlight.
      editor.onDidScrollChange((e: any) => {
        const bus = syncRef.current;
        if (bus.lock) return;
        const self = bus[side];
        const peer = side === "left" ? bus.right : bus.left;
        if (!self || !peer) return;
        const delta = e.scrollTop - self.anchor;
        const target = Math.max(0, peer.anchor + delta);
        bus.lock = true;
        try {
          peer.editor.setScrollTop(target, 1 /* Immediate */);
          peer.editor.setScrollLeft(e.scrollLeft, 1);
        } finally {
          bus.lock = false;
        }
      });
      setEditorReady(true);
    })();
    return () => {
      cancelled = true;
      const editor = editorRef.current;
      const model = modelRef.current;
      editorRef.current = null;
      modelRef.current = null;
      if (syncRef.current[side]?.editor === editor) {
        syncRef.current[side] = null;
      }
      editor?.setModel(null);
      editor?.dispose();
      model?.dispose();
    };
  }, [side, syncRef]);

  useEffect(() => {
    if (!editorReady) return;
    let cancelled = false;
    (async () => {
      try {
        const content = await readAt(stream.id, path, version);
        if (cancelled) return;
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (!monaco || !editor) return;
        const language = languageForPath(path) ?? "plaintext";
        const model = monaco.editor.createModel(content ?? "", language);
        const previous = modelRef.current;
        editor.setModel(model);
        modelRef.current = model;
        previous?.dispose();

        const lineCount = model.getLineCount();
        const safeStart = Math.max(1, Math.min(startLine, lineCount));
        const safeEnd = Math.max(safeStart, Math.min(endLine, lineCount));
        editor.deltaDecorations(
          [],
          [
            {
              range: new monaco.Range(safeStart, 1, safeEnd, model.getLineMaxColumn(safeEnd)),
              options: {
                isWholeLine: true,
                className: "oxplow-duplicate-block-line",
                marginClassName: "oxplow-duplicate-block-margin",
              },
            },
          ],
        );
        // Record this side's pixel-top-for-start in the bus, then
        // ask `recomputeAlignment` to pick a shared pad both sides
        // can satisfy and apply it. When only one side is loaded
        // we align that side immediately; when the second side
        // arrives, recompute also pulls the first side back into
        // alignment if the pad shrunk.
        const lineHeight: number = editor.getOption(
          monaco.editor.EditorOption.lineHeight,
        );
        const top: number = editor.getTopForLineNumber(safeStart);
        const bus = syncRef.current;
        if (bus[side]) {
          bus[side]!.topForStart = top;
          bus[side]!.lineHeight = lineHeight;
        }
        recomputeAlignment(bus);
        editor.setPosition({ lineNumber: safeStart, column: 1 });
        setError(null);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [
    editorReady,
    stream.id,
    path,
    version,
    startLine,
    endLine,
  ]);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0, minWidth: 0 }}>
      <div
        style={{
          padding: "4px 10px",
          borderBottom: "1px solid var(--border-subtle)",
          fontSize: 11,
          display: "flex",
          gap: 8,
          alignItems: "center",
        }}
      >
        <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--text-primary)" }}>
          {path}
        </span>
        <span style={{ color: "var(--text-muted)" }}>
          :{startLine}-{endLine}
        </span>
        <span
          style={{
            fontFamily: "ui-monospace, monospace",
            color: "var(--text-muted)",
            border: "1px solid var(--border-subtle)",
            borderRadius: 3,
            padding: "0 6px",
          }}
          title={`Version: ${versionLabel}`}
        >
          @{versionLabel}
        </span>
        {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : null}
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={() => onJumpToSource(path, version)}
          style={{
            background: "var(--surface-card)",
            color: "var(--text-primary)",
            borderWidth: 1,
            borderStyle: "solid",
            borderColor: "var(--border-subtle)",
            borderRadius: 3,
            padding: "2px 8px",
            fontSize: 11,
            cursor: "pointer",
          }}
        >
          Open file
        </button>
      </div>
      <div ref={hostRef} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
}
