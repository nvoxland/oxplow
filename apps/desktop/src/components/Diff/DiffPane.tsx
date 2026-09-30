import React, { useEffect, useRef, useState } from "react";
import { readAt, type Stream } from "../../api.js";
import { languageForPath } from "../../editor-language.js";
import type { Revision } from "../../revision.js";

export interface DiffSpec {
  path: string;
  /** Version to load on the LEFT side of the diff. Required — there
   *  is no implicit "current working tree" default. */
  leftVersion: Revision;
  /** Version to load on the RIGHT side. */
  rightVersion: Revision;
  baseLabel: string;
  /** When set, skip reading the left side and diff this literal text instead. */
  leftContent?: string;
  /** When set, skip reading the right side and diff this literal text instead. */
  rightContent?: string;
  /** Optional override for the tab label suffix shown next to the filename. */
  labelOverride?: string;
  /** When set, scroll/reveal this 1-based line on the modified (right)
   *  side after both models load, and select it so the position is
   *  obvious. Used by the Change Analysis function rows to land on the
   *  function's start line. */
  revealLine?: number;
}

interface Props {
  stream: Stream;
  spec: DiffSpec;
  visible: boolean;
  /** Open the right-side path in the regular editor pane and close this diff tab. */
  onJumpToSource?: (path: string) => void;
}

const toolbarButtonStyle: React.CSSProperties = {
  background: "var(--panel)",
  color: "var(--fg)",
  border: "1px solid var(--border)",
  borderRadius: 3,
  padding: "2px 8px",
  fontSize: 11,
  cursor: "pointer",
};

export function DiffPane({ stream, spec, visible, onJumpToSource }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelsRef = useRef<{ left: any; right: any } | null>(null);
  const monacoRef = useRef<any>(null);
  const [editorReady, setEditorReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // Set of modified-side line numbers that begin a diff change. Combined
  // with the modified editor's cursor line below, these drive the
  // enabled/disabled state of the Prev/Next change buttons. We keep just
  // the start lines (sorted ascending) — Monaco's `goToDiff` lands the
  // cursor on a change start, so checking "is there a change strictly
  // before/after the cursor line" is sufficient.
  const [changeStarts, setChangeStarts] = useState<number[]>([]);
  const [cursorLine, setCursorLine] = useState<number>(1);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      monacoRef.current = monaco;
      const editor = monaco.editor.createDiffEditor(hostRef.current, {
        automaticLayout: true,
        readOnly: true,
        renderSideBySide: true,
        theme: "vs-dark",
        minimap: { enabled: false },
      });
      editorRef.current = editor;
      setEditorReady(true);
    })();
    return () => {
      cancelled = true;
      const editor = editorRef.current;
      const models = modelsRef.current;
      editorRef.current = null;
      modelsRef.current = null;
      editor?.setModel(null);
      editor?.dispose();
      models?.left?.dispose();
      models?.right?.dispose();
    };
  }, []);

  useEffect(() => {
    if (!stream || !editorReady) return;
    let cancelled = false;
    (async () => {
      try {
        const leftPromise = spec.leftContent !== undefined
          ? Promise.resolve(spec.leftContent as string | null)
          : readAt(stream.id, spec.path, spec.leftVersion);
        const rightPromise = spec.rightContent !== undefined
          ? Promise.resolve(spec.rightContent as string | null)
          : readAt(stream.id, spec.path, spec.rightVersion);
        const [leftContent, rightContent] = await Promise.all([leftPromise, rightPromise]);
        if (cancelled) return;
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (!monaco || !editor) return;
        const language = languageForPath(spec.path) ?? "plaintext";
        const left = monaco.editor.createModel(leftContent ?? "", language);
        const right = monaco.editor.createModel(rightContent ?? "", language);
        const previous = modelsRef.current;
        editor.setModel({ original: left, modified: right });
        modelsRef.current = { left, right };
        previous?.left?.dispose();
        previous?.right?.dispose();
        setError(null);
        if (spec.revealLine && spec.revealLine > 0) {
          // Reveal on the modified (right) editor, which is what the
          // analysis-page jump targets. The diff editor exposes the
          // sub-editor via getModifiedEditor(); fall back gracefully
          // if Monaco's API ever shifts shape.
          const modifiedEditor = (editor as { getModifiedEditor?: () => any }).getModifiedEditor?.();
          if (modifiedEditor) {
            const line = spec.revealLine;
            modifiedEditor.revealLineInCenter(line);
            modifiedEditor.setPosition({ lineNumber: line, column: 1 });
            modifiedEditor.setSelection({ startLineNumber: line, startColumn: 1, endLineNumber: line, endColumn: 1 });
            modifiedEditor.focus();
          }
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [
    stream,
    editorReady,
    spec.path,
    spec.leftVersion,
    spec.rightVersion,
    spec.leftContent,
    spec.rightContent,
    spec.revealLine,
  ]);

  // Refresh the change list whenever Monaco recomputes the diff, and
  // track the modified editor's cursor line so Prev/Next can be
  // disabled when there's no change before/after the cursor.
  useEffect(() => {
    if (!editorReady) return;
    const editor = editorRef.current as
      | {
          onDidUpdateDiff?: (cb: () => void) => { dispose(): void };
          getLineChanges?: () => Array<{ modifiedStartLineNumber: number }> | null;
          getDiffComputationResult?: () => {
            changes: Array<{ modified: { startLineNumber: number } }>;
          } | null;
          getModifiedEditor?: () => {
            getPosition?: () => { lineNumber: number } | null;
            onDidChangeCursorPosition?: (cb: (e: { position: { lineNumber: number } }) => void) => {
              dispose(): void;
            };
          };
        }
      | null;
    if (!editor) return;
    const refreshChanges = () => {
      const legacy = editor.getLineChanges?.();
      if (legacy) {
        setChangeStarts(legacy.map((c) => c.modifiedStartLineNumber).sort((a, b) => a - b));
        return;
      }
      const computed = editor.getDiffComputationResult?.();
      if (computed) {
        setChangeStarts(
          computed.changes.map((c) => c.modified.startLineNumber).sort((a, b) => a - b),
        );
      }
    };
    refreshChanges();
    const diffSub = editor.onDidUpdateDiff?.(refreshChanges);
    const modified = editor.getModifiedEditor?.();
    const initialPos = modified?.getPosition?.();
    if (initialPos) setCursorLine(initialPos.lineNumber);
    const cursorSub = modified?.onDidChangeCursorPosition?.((e) => {
      setCursorLine(e.position.lineNumber);
    });
    return () => {
      diffSub?.dispose();
      cursorSub?.dispose();
    };
  }, [editorReady]);

  const hasPrevChange = changeStarts.some((line) => line < cursorLine);
  const hasNextChange = changeStarts.some((line) => line > cursorLine);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div style={{ padding: "4px 10px", borderBottom: "1px solid var(--border)", color: "var(--muted)", fontSize: 11, display: "flex", gap: 8, alignItems: "center" }}>
        <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--fg)" }}>{spec.path}</span>
        <span>vs {spec.baseLabel}</span>
        {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : null}
        <span style={{ flex: 1 }} />
        <button
          type="button"
          onClick={() => editorRef.current?.goToDiff("previous")}
          disabled={!editorReady || !hasPrevChange}
          title="Previous change"
          data-testid="diff-prev-change"
          style={toolbarButtonStyle}
        >
          ↑ Prev
        </button>
        <button
          type="button"
          onClick={() => editorRef.current?.goToDiff("next")}
          disabled={!editorReady || !hasNextChange}
          title="Next change"
          data-testid="diff-next-change"
          style={toolbarButtonStyle}
        >
          ↓ Next
        </button>
        <button
          type="button"
          onClick={() => onJumpToSource?.(spec.path)}
          disabled={!onJumpToSource}
          title="Open file in editor"
          data-testid="diff-jump-to-source"
          style={toolbarButtonStyle}
        >
          Open file
        </button>
      </div>
      <div
        ref={hostRef}
        style={{
          flex: 1,
          minHeight: 0,
          display: visible ? "block" : "none",
        }}
      />
    </div>
  );
}
