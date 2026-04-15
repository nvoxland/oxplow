import type { CSSProperties } from "react";
import { useEffect, useRef } from "react";
import type { OpenFileState } from "../../file-session.js";
import type { Stream } from "../api.js";

interface Props {
  stream: Stream;
  filePath: string | null;
  value: string;
  isDirty: boolean;
  isLoading: boolean;
  onChange(value: string): void;
  onSave(): void;
  openFileOrder: string[];
  openFiles: Record<string, OpenFileState>;
  onSelectOpenFile(path: string): void;
  onCloseOpenFile(path: string): void;
}

export function EditorPane({
  stream,
  filePath,
  value,
  isDirty,
  isLoading,
  onChange,
  onSave,
  openFileOrder,
  openFiles,
  onSelectOpenFile,
  onCloseOpenFile,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const changeDisposeRef = useRef<{ dispose(): void } | null>(null);
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);

  onChangeRef.current = onChange;
  onSaveRef.current = onSave;

  useEffect(() => {
    let cancelled = false;
    (async () => {
      const monaco = await import("monaco-editor");
      if (cancelled || !hostRef.current) return;
      monacoRef.current = monaco;
      const editor = monaco.editor.create(hostRef.current, {
        value: "",
        language: "typescript",
        theme: "vs-dark",
        automaticLayout: true,
        minimap: { enabled: false },
      });
      editor.addAction({
        id: "newde-save-file",
        label: "Save File",
        keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS],
        run: () => onSaveRef.current(),
      });
      editorRef.current = editor;
    })();
    return () => {
      cancelled = true;
      changeDisposeRef.current?.dispose();
      editorRef.current?.dispose();
    };
  }, []);

  useEffect(() => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;

    const uri = filePath
      ? monaco.Uri.from({ scheme: "file", path: `/${stream.id}/${filePath}` })
      : monaco.Uri.from({ scheme: "inmemory", path: `/stream/${stream.id}/placeholder` });
    let model = monaco.editor.getModel(uri);
    if (!model) {
      model = monaco.editor.createModel(value, languageForPath(filePath), uri);
    }
    if (model.getValue() !== value) {
      model.setValue(value);
    }
    monaco.editor.setModelLanguage(model, languageForPath(filePath));
    editor.setModel(model);
    changeDisposeRef.current?.dispose();
    changeDisposeRef.current = editor.onDidChangeModelContent(() => {
      const next = editor.getValue();
      if (next !== value) onChangeRef.current(next);
    });
  }, [stream.id, filePath, value]);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div
        style={{
          display: "flex",
          justifyContent: "space-between",
          alignItems: "center",
          padding: "8px 12px",
          borderBottom: "1px solid var(--border)",
          fontSize: 12,
          color: "var(--muted)",
        }}
      >
        <span>{filePath ? `${filePath}${isDirty ? " • modified" : ""}` : "No file selected"}</span>
        <button
          onClick={onSave}
          disabled={!filePath || isLoading || !isDirty}
          style={{
            background: "var(--bg-2)",
            color: "var(--fg)",
            border: "1px solid var(--border)",
            padding: "4px 10px",
            borderRadius: 4,
            cursor: isLoading || !isDirty ? "default" : "pointer",
            fontFamily: "inherit",
          }}
        >
          {isLoading ? "Saving…" : "Save"}
        </button>
      </div>
      {openFileOrder.length > 0 ? (
        <div
          style={{
            display: "flex",
            gap: 4,
            padding: "6px 8px",
            borderBottom: "1px solid var(--border)",
            background: "var(--bg-2)",
            overflowX: "auto",
          }}
        >
          {openFileOrder.map((path) => {
            const openFile = openFiles[path];
            const active = path === filePath;
            const dirty = !!openFile && openFile.draftContent !== openFile.savedContent;
            return (
              <div
                key={path}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  border: "1px solid var(--border)",
                  borderBottom: active ? "2px solid var(--accent)" : "2px solid transparent",
                  background: active ? "var(--bg)" : "transparent",
                  borderRadius: 4,
                  padding: "4px 8px",
                  flexShrink: 0,
                }}
              >
                <button
                  onClick={() => onSelectOpenFile(path)}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: active ? "var(--fg)" : "var(--muted)",
                    cursor: "pointer",
                    fontFamily: "inherit",
                  }}
                >
                  {basename(path)}{dirty ? " •" : ""}
                </button>
                <button
                  onClick={() => onCloseOpenFile(path)}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: "var(--muted)",
                    cursor: "pointer",
                    fontFamily: "inherit",
                    padding: 0,
                  }}
                  aria-label={`Close ${path}`}
                >
                  ×
                </button>
              </div>
            );
          })}
        </div>
      ) : null}
      <div style={{ position: "relative", width: "100%", height: "100%" }}>
        <div ref={hostRef} style={{ width: "100%", height: "100%" }} />
        {!filePath ? (
          <div style={emptyStyle}>
            <div>Select a file from the sidebar to open it here.</div>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function basename(path: string): string {
  const parts = path.split("/");
  return parts[parts.length - 1] ?? path;
}

const emptyStyle: CSSProperties = {
  position: "absolute",
  inset: 0,
  display: "flex",
  alignItems: "center",
  justifyContent: "center",
  color: "var(--muted)",
  fontSize: 13,
  background: "rgba(14, 14, 14, 0.85)",
};

function languageForPath(path: string | null): string {
  if (!path) return "plaintext";
  if (path.endsWith(".ts")) return "typescript";
  if (path.endsWith(".tsx")) return "typescript";
  if (path.endsWith(".js") || path.endsWith(".jsx")) return "javascript";
  if (path.endsWith(".json")) return "json";
  if (path.endsWith(".md")) return "markdown";
  if (path.endsWith(".css")) return "css";
  if (path.endsWith(".html")) return "html";
  if (path.endsWith(".yml") || path.endsWith(".yaml")) return "yaml";
  if (path.endsWith(".sh")) return "shell";
  return "plaintext";
}
