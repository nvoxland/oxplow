import { useEffect, useRef, useState } from "react";
import { readFileAtRef, readWorkspaceFile, type Stream } from "../../api.js";
import { languageForPath } from "../../editor-language.js";

export interface DiffSpec {
  path: string;
  leftRef: string;
  rightKind: "working" | { ref: string };
  baseLabel: string;
  /** When set, skip reading the left side and diff this literal text instead. */
  leftContent?: string;
  /** When set, skip reading the right side and diff this literal text instead. */
  rightContent?: string;
  /** Optional override for the tab label suffix shown next to the filename. */
  labelOverride?: string;
}

interface Props {
  stream: Stream;
  spec: DiffSpec;
  visible: boolean;
}

export function DiffPane({ stream, spec, visible }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelsRef = useRef<{ left: any; right: any } | null>(null);
  const monacoRef = useRef<any>(null);
  const [error, setError] = useState<string | null>(null);

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
    })();
    return () => {
      cancelled = true;
      editorRef.current?.dispose();
      editorRef.current = null;
      if (modelsRef.current) {
        modelsRef.current.left?.dispose();
        modelsRef.current.right?.dispose();
        modelsRef.current = null;
      }
    };
  }, []);

  useEffect(() => {
    if (!stream) return;
    let cancelled = false;
    (async () => {
      try {
        const leftPromise = spec.leftContent !== undefined
          ? Promise.resolve({ content: spec.leftContent })
          : readFileAtRef(stream.id, spec.leftRef, spec.path);
        const rightPromise = spec.rightContent !== undefined
          ? Promise.resolve({ content: spec.rightContent })
          : spec.rightKind === "working"
            ? readWorkspaceFile(stream.id, spec.path).then(
                (file) => ({ content: file.content as string | null }),
                () => ({ content: null as string | null }),
              )
            : readFileAtRef(stream.id, spec.rightKind.ref, spec.path);
        const [leftResult, rightResult] = await Promise.all([leftPromise, rightPromise]);
        if (cancelled) return;
        const monaco = monacoRef.current;
        const editor = editorRef.current;
        if (!monaco || !editor) return;
        if (modelsRef.current) {
          modelsRef.current.left?.dispose();
          modelsRef.current.right?.dispose();
        }
        const language = languageForPath(spec.path) ?? "plaintext";
        const left = monaco.editor.createModel(leftResult.content ?? "", language);
        const right = monaco.editor.createModel(rightResult.content ?? "", language);
        editor.setModel({ original: left, modified: right });
        modelsRef.current = { left, right };
        setError(null);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => { cancelled = true; };
  }, [stream, spec.path, spec.leftRef, typeof spec.rightKind === "string" ? "working" : spec.rightKind.ref, spec.leftContent, spec.rightContent]);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
      <div style={{ padding: "4px 10px", borderBottom: "1px solid var(--border)", color: "var(--muted)", fontSize: 11, display: "flex", gap: 8, alignItems: "center" }}>
        <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--fg)" }}>{spec.path}</span>
        <span>vs {spec.baseLabel}</span>
        {error ? <span style={{ color: "#ff6b6b" }}>{error}</span> : null}
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
