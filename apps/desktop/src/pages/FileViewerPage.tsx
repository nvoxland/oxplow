import React, { useEffect, useRef, useState } from "react";
import { Page } from "../tabs/Page.js";
import { usePageTitle } from "../tabs/PageNavigationContext.js";
import { readAt, type Stream } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { shortRevisionLabel, type Revision } from "../revision.js";

export interface FileViewerPageProps {
  stream: Stream;
  path: string;
  /** Non-disk version. Disk-version files go through `FilePage` /
   *  `EditorPane` instead because that pipeline owns dirty state +
   *  saves. */
  version: Revision;
  visible: boolean;
}

/**
 * Read-only viewer for a file at a non-disk version (a git ref or a
 * snapshot). Loads via `readAt(streamId, path, version)` and
 * renders a read-only Monaco editor with a banner that names the
 * version. Keeps the EditorPane / save pipeline disk-only.
 *
 * This page exists because making EditorPane version-aware would
 * splice "is this read-only" branches throughout the dirty-state
 * cache, the LSP bridge, the find-in-file flow, and the context
 * menus. The duplication of one Monaco mount is cheap by comparison.
 */
export function FileViewerPage({ stream, path, version, visible }: FileViewerPageProps) {
  const basename = path.split("/").pop() ?? path;
  const versionLabel = shortRevisionLabel(version);
  usePageTitle(`${basename} (${versionLabel})`);

  void visible;

  return (
    <Page testId="page-file-viewer" kind="file">
      <div style={{ display: "flex", flexDirection: "column", flex: 1, minHeight: 0 }}>
        <div
          style={{
            padding: "4px 10px",
            borderBottom: "1px solid var(--border-subtle)",
            color: "var(--text-muted)",
            fontSize: 11,
            display: "flex",
            gap: 8,
            alignItems: "center",
          }}
        >
          <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--text-primary)" }}>
            {path}
          </span>
          <span>at</span>
          <span style={{ fontFamily: "ui-monospace, monospace", color: "var(--text-primary)" }}>
            {versionLabel}
          </span>
          <span style={{ flex: 1 }} />
          <span style={{ color: "var(--text-muted)" }}>read-only</span>
        </div>
        <ViewerBody stream={stream} path={path} version={version} />
      </div>
    </Page>
  );
}

interface BodyProps {
  stream: Stream;
  path: string;
  version: Revision;
}

function ViewerBody({ stream, path, version }: BodyProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const modelRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const [editorReady, setEditorReady] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
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
        scrollBeyondLastLine: false,
      });
      editorRef.current = editor;
      setEditorReady(true);
    })();
    return () => {
      cancelled = true;
      const editor = editorRef.current;
      const model = modelRef.current;
      editorRef.current = null;
      modelRef.current = null;
      editor?.setModel(null);
      editor?.dispose();
      model?.dispose();
    };
  }, []);

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
        if (content == null) {
          setError(`File does not exist at ${shortRevisionLabel(version)}`);
        } else {
          setError(null);
        }
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
  ]);

  return (
    <div style={{ flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}>
      {error ? (
        <div style={{ padding: "4px 10px", color: "#ff6b6b", fontSize: 11, borderBottom: "1px solid var(--border-subtle)" }}>
          {error}
        </div>
      ) : null}
      <div ref={hostRef} style={{ flex: 1, minHeight: 0 }} />
    </div>
  );
}
