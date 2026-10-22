import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";

import { readAt } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { shortRevisionLabel, type Revision } from "../revision.js";
import { useMonacoViewState } from "../components/useMonacoViewState.js";
import { useOptionalPageNavigation } from "../tabs/PageNavigationContext.js";
import { fileRef } from "../tabs/pageRefs.js";
import type { TabRef } from "../tabs/tabState.js";
import { ScrollSync } from "./codeScrollSync.js";
import type { codeRows } from "./lensModel.js";

type CodeRow = ReturnType<typeof codeRows>[number];

/**
 * `code`: each row's file, read-only, at its revision — side by side, at
 * the height its place gives (a page's whole body on a lens page). A row's
 * `lines` are marked and revealed; with `sync` the editors scroll
 * together from their marks (`codeScrollSync.ts`). See
 * `.context/editor-and-monaco.md` "Read-only code and diff vizes".
 */
export function CodeViz({ rows, streamId, sync, onOpenPage }: { rows: CodeRow[]; streamId: string | null; sync: boolean; onOpenPage?(ref: TabRef): void }) {
  const bus = useMemo(() => (sync ? new ScrollSync() : null), [sync]);
  return (
    <div data-testid="lens-code" style={rowStyle}>
      {rows.map((r, i) => (
        <CodeEditorView key={`${i}:${r.path}@${r.revision}`} syncKey={String(i)} row={r} streamId={streamId} sync={bus} onOpenPage={onOpenPage} divider={i > 0} />
      ))}
    </div>
  );
}

/** The one read-only Monaco mount: a file at a revision, its lines marked. */
function CodeEditorView({
  row,
  streamId,
  sync,
  syncKey,
  onOpenPage,
  divider,
}: {
  row: CodeRow;
  streamId: string | null;
  sync: ScrollSync | null;
  syncKey: string;
  onOpenPage?(ref: TabRef): void;
  divider: boolean;
}) {
  const { path, revision, lines } = row;
  const nav = useOptionalPageNavigation();
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const [ready, setReady] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  // Scroll and cursor stay with the tab's entry, unless lines are marked:
  // the mark is where the page lands.
  const attachViewState = useMonacoViewState(`code:${path}@${revision}`);

  useEffect(() => {
    let cancelled = false;
    let off: (() => void) | null = null;
    void (async () => {
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
      if (sync) {
        const unregister = sync.register(syncKey, editor);
        const sub = editor.onDidScrollChange((e: { scrollTop: number; scrollLeft: number }) => sync.scrolled(syncKey, e.scrollTop, e.scrollLeft));
        off = () => {
          sub.dispose();
          unregister();
        };
      }
      setReady(true);
    })();
    return () => {
      cancelled = true;
      off?.();
      const editor = editorRef.current;
      const model = editor?.getModel?.();
      editorRef.current = null;
      editor?.setModel(null);
      editor?.dispose();
      model?.dispose();
    };
  }, [sync, syncKey]);

  useEffect(() => {
    if (!ready) return;
    let cancelled = false;
    let detach: (() => void) | null = null;
    void (async () => {
      try {
        const content = await readAt(streamId ?? "", path, revision as Revision);
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (cancelled || !monaco || !editor) return;
        const model = monaco.editor.createModel(content ?? "", languageForPath(path) ?? "plaintext");
        const previous = editor.getModel?.();
        editor.setModel(model);
        previous?.dispose();
        setProblem(content === null ? `Not there at ${shortRevisionLabel(revision)}` : null);
        if (!lines) {
          detach = attachViewState(editor);
          return;
        }
        const last = model.getLineCount();
        const start = Math.min(lines.start, last);
        const end = Math.max(start, Math.min(lines.end, last));
        editor.deltaDecorations(
          [],
          [
            {
              range: new monaco.Range(start, 1, end, model.getLineMaxColumn(end)),
              options: { isWholeLine: true, className: "ox-code-mark", marginClassName: "ox-code-mark-margin" },
            },
          ],
        );
        editor.setPosition({ lineNumber: start, column: 1 });
        if (sync) sync.place(syncKey, editor.getTopForLineNumber(start), editor.getOption(monaco.editor.EditorOption.lineHeight));
        else editor.revealLineNearTop(start);
      } catch (e) {
        if (!cancelled) setProblem(e instanceof Error ? e.message : String(e));
      }
    })();
    return () => {
      cancelled = true;
      detach?.();
    };
    // `lines` stands in by its ends (a fresh object each run).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, streamId, path, revision, lines?.start, lines?.end, sync, syncKey]);

  const openFile = () => {
    const ref = fileRef(path);
    if (nav) nav.navigate(ref);
    else onOpenPage?.(ref);
  };

  return (
    <div data-testid="lens-code-file" style={{ ...sideStyle, borderLeft: divider ? "1px solid var(--border-subtle)" : undefined }}>
      <div style={stripStyle}>
        <span data-testid="lens-code-path" style={{ fontFamily: "var(--font-mono)", color: "var(--text-primary)" }}>
          {path}
        </span>
        {lines ? <span>{lines.start === lines.end ? `:${lines.start}` : `:${lines.start}-${lines.end}`}</span> : null}
        <span title={`Revision: ${revision}`} style={{ fontFamily: "var(--font-mono)" }}>
          @{shortRevisionLabel(revision)}
        </span>
        {problem ? (
          <span data-testid="lens-code-problem" style={{ color: "var(--severity-critical)" }}>
            {problem}
          </span>
        ) : null}
        <span style={{ flex: 1 }} />
        <button type="button" className="ox-button" data-testid="lens-code-open" onClick={openFile}>
          Open file
        </button>
      </div>
      <div ref={hostRef} style={{ flex: 1, minHeight: 0 }} />
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
