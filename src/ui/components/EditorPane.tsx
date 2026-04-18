import type { CSSProperties } from "react";
import type { MutableRefObject } from "react";
import { useEffect, useRef, useState } from "react";
import type { OpenFileState } from "../../session/file-session.js";
import type { BlameLine, Stream } from "../api.js";
import { gitBlame } from "../api.js";
import { isLspCandidateLanguage, languageForPath } from "../editor-language.js";
import { LspClient, type EditorNavigationTarget, streamFileUri, toEditorNavigationTarget } from "../lsp.js";
import type { MenuItem } from "../menu.js";
import { ContextMenu } from "./ContextMenu.js";

interface Props {
  stream: Stream;
  filePath: string | null;
  value: string;
  isDirty: boolean;
  onChange(value: string): void;
  onSave(): void | Promise<void>;
  findRequest: number;
  navigationTarget: EditorNavigationTarget | null;
  openFileOrder: string[];
  openFiles: Record<string, OpenFileState>;
  onNavigateToLocation(target: EditorNavigationTarget): Promise<void>;
  onSelectOpenFile(path: string): void;
  onCloseOpenFile(path: string): void;
  onRevealCommit?(sha: string): void;
}

export function EditorPane({
  stream,
  filePath,
  value,
  isDirty,
  onChange,
  onSave,
  findRequest,
  navigationTarget,
  openFileOrder,
  openFiles,
  onNavigateToLocation,
  onSelectOpenFile,
  onCloseOpenFile,
  onRevealCommit,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const changeDisposeRef = useRef<{ dispose(): void } | null>(null);
  const focusDisposersRef = useRef<{ dispose(): void }[]>([]);
  const focusDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const openFilesRef = useRef({ order: openFileOrder, files: openFiles });
  const onChangeRef = useRef(onChange);
  const onNavigateRef = useRef(onNavigateToLocation);
  const streamRef = useRef(stream);
  const filePathRef = useRef(filePath);
  const lspClientsRef = useRef(new Map<string, LspClient>());
  const trackedOpenDocsRef = useRef(new Map<string, string>());
  const trackedSavedContentRef = useRef(new Map<string, string>());
  const diagnosticsDisposersRef = useRef<(() => void)[]>([]);
  const markerOwnerRef = useRef(`newde-lsp-${stream.id}`);
  const [lspStatus, setLspStatus] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [blame, setBlame] = useState<{ path: string; lines: BlameLine[] } | null>(null);
  const [blameScrollTop, setBlameScrollTop] = useState(0);
  const [blameLineHeight, setBlameLineHeight] = useState(19);
  const prevDirtyRef = useRef(isDirty);
  const onRevealCommitRef = useRef(onRevealCommit);
  onRevealCommitRef.current = onRevealCommit;
  // Monaco loads asynchronously, so the model-binding effect below needs a
  // signal to retry once it lands — otherwise the first file opened arrives
  // before the editor instance exists and the effect's early return makes
  // the pane stay blank until the next file change.
  const [monacoReady, setMonacoReady] = useState(false);

  onChangeRef.current = onChange;
  onNavigateRef.current = onNavigateToLocation;
  streamRef.current = stream;
  filePathRef.current = filePath;
  openFilesRef.current = { order: openFileOrder, files: openFiles };

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
        contextmenu: false,
      });
      editorRef.current = editor;
      registerGoToDefinitionAction(monaco, editor, () => goToDefinition());
      registerLspProviders(monaco, (languageId) => ensureLspClient(streamRef.current, languageId), streamRef);
      focusDisposersRef.current.push(editor.onDidChangeCursorSelection(() => scheduleFocusPush()));
      focusDisposersRef.current.push(editor.onDidChangeCursorPosition(() => scheduleFocusPush()));
      editor.onContextMenu((event: any) => {
        if (!filePathRef.current) return;
        const position = event.target?.position ?? editor.getPosition();
        if (position) {
          editor.setPosition(position);
        }
        editor.focus();
        const browserEvent = event.event?.browserEvent as MouseEvent | undefined;
        setContextMenu({
          x: browserEvent?.clientX ?? 8,
          y: browserEvent?.clientY ?? 8,
        });
      });
      setMonacoReady(true);
    })();
    return () => {
      cancelled = true;
      changeDisposeRef.current?.dispose();
      diagnosticsDisposersRef.current.forEach((dispose) => dispose());
      diagnosticsDisposersRef.current = [];
      focusDisposersRef.current.forEach((d) => d.dispose());
      focusDisposersRef.current = [];
      if (focusDebounceRef.current) clearTimeout(focusDebounceRef.current);
      for (const client of lspClientsRef.current.values()) {
        client.dispose();
      }
      lspClientsRef.current.clear();
      editorRef.current?.dispose();
    };
  }, []);

  function scheduleFocusPush() {
    if (focusDebounceRef.current) clearTimeout(focusDebounceRef.current);
    focusDebounceRef.current = setTimeout(() => {
      focusDebounceRef.current = null;
      pushEditorFocus();
    }, 150);
  }

  function pushEditorFocus() {
    const editor = editorRef.current;
    if (!editor) return;
    const currentStream = streamRef.current;
    const currentPath = filePathRef.current;
    const { order, files } = openFilesRef.current;
    const selectionObj = editor.getSelection?.();
    const hasSelection = !!selectionObj && !selectionObj.isEmpty();
    let selection: {
      startLine: number;
      startColumn: number;
      endLine: number;
      endColumn: number;
      text: string;
    } | null = null;
    let caret: { line: number; column: number } | null = null;
    if (hasSelection) {
      const model = editor.getModel?.();
      const text: string = model ? model.getValueInRange(selectionObj) : "";
      selection = {
        startLine: selectionObj.startLineNumber,
        startColumn: selectionObj.startColumn,
        endLine: selectionObj.endLineNumber,
        endColumn: selectionObj.endColumn,
        text: text.length > 20_000 ? `${text.slice(0, 20_000)}…` : text,
      };
    } else {
      const position = editor.getPosition?.();
      if (position) caret = { line: position.lineNumber, column: position.column };
    }
    const openFilesPayload = order
      .map((path) => {
        const entry = files[path];
        if (!entry) return null;
        return { path, dirty: entry.draftContent !== entry.savedContent };
      })
      .filter((entry): entry is { path: string; dirty: boolean } => !!entry);
    const api = (window as any).newdeApi as { updateEditorFocus?: (payload: unknown) => unknown } | undefined;
    if (!api?.updateEditorFocus) return;
    void api.updateEditorFocus({
      streamId: currentStream.id,
      activeFile: currentPath,
      caret,
      selection,
      openFiles: openFilesPayload,
    });
  }

  useEffect(() => {
    setContextMenu(null);
  }, [filePath, stream.id]);

  useEffect(() => {
    pushEditorFocus();
  }, [stream.id, filePath, openFileOrder, openFiles]);

  useEffect(() => {
    const monaco = monacoRef.current;
    const editor = editorRef.current;
    if (!monaco || !editor) return;

    const uri = filePath
      ? monaco.Uri.parse(streamFileUri(stream, filePath))
      : monaco.Uri.from({ scheme: "inmemory", path: `/stream/${stream.id}/placeholder` });
    let model = monaco.editor.getModel(uri);
    if (!model) {
      model = monaco.editor.createModel(value, languageForPath(filePath), uri);
    }
    if (model.getValue() !== value) {
      model.setValue(value);
    }
    monaco.editor.setModelLanguage(model, languageForPath(filePath));
    closeFindWidget(editor);
    editor.setModel(model);
    changeDisposeRef.current?.dispose();
    changeDisposeRef.current = editor.onDidChangeModelContent(() => {
      const next = editor.getValue();
      if (next !== value) onChangeRef.current(next);
      const currentModel = editor.getModel();
      if (!currentModel || !filePathRef.current) return;
      const currentLanguage = currentModel.getLanguageId();
      if (!isLspCandidateLanguage(currentLanguage)) return;
      ensureLspClient(streamRef.current, currentLanguage).notify("textDocument/didChange", {
        textDocument: {
          uri: currentModel.uri.toString(),
          version: currentModel.getVersionId(),
        },
        contentChanges: [{ text: next }],
      });
    });
  }, [stream.id, filePath, value, monacoReady]);

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !filePath || findRequest === 0) return;
    void editor.getAction("actions.find")?.run();
  }, [filePath, findRequest]);

  useEffect(() => {
    const monaco = monacoRef.current;
    if (!monaco) return;
    const nextOpenDocs = new Map<string, string>();
    for (const path of openFileOrder) {
      const openFile = openFiles[path];
      if (!openFile || openFile.isLoading) continue;
      const languageId = languageForPath(path);
      if (!isLspCandidateLanguage(languageId)) continue;
      const uri = streamFileUri(stream, path);
      nextOpenDocs.set(path, languageId);
      if (!trackedOpenDocsRef.current.has(path)) {
        ensureLspClient(stream, languageId).notify("textDocument/didOpen", {
          textDocument: {
            uri,
            languageId,
            version: 1,
            text: openFile.draftContent,
          },
        });
      }
      trackedSavedContentRef.current.set(path, openFile.savedContent);
    }

    for (const [path, languageId] of trackedOpenDocsRef.current) {
      if (nextOpenDocs.has(path)) continue;
      ensureLspClient(stream, languageId).notify("textDocument/didClose", {
        textDocument: { uri: streamFileUri(stream, path) },
      });
      const model = monaco.editor.getModel(monaco.Uri.parse(streamFileUri(stream, path)));
      if (model) {
        monaco.editor.setModelMarkers(model, markerOwnerRef.current, []);
      }
      trackedSavedContentRef.current.delete(path);
    }

    trackedOpenDocsRef.current = nextOpenDocs;
  }, [openFileOrder, openFiles, stream]);

  useEffect(() => {
    for (const [path, languageId] of trackedOpenDocsRef.current) {
      const openFile = openFiles[path];
      const previousSaved = trackedSavedContentRef.current.get(path);
      if (!openFile || previousSaved === undefined || previousSaved === openFile.savedContent) continue;
      trackedSavedContentRef.current.set(path, openFile.savedContent);
      if (openFile.savedContent !== openFile.draftContent) continue;
      ensureLspClient(stream, languageId).notify("textDocument/didSave", {
        textDocument: { uri: streamFileUri(stream, path) },
        text: openFile.savedContent,
      });
    }
  }, [openFiles, stream]);

  useEffect(() => {
    markerOwnerRef.current = `newde-lsp-${stream.id}`;
    setLspStatus(null);
    for (const client of lspClientsRef.current.values()) {
      client.dispose();
    }
    lspClientsRef.current.clear();
    trackedOpenDocsRef.current.clear();
    trackedSavedContentRef.current.clear();
    diagnosticsDisposersRef.current.forEach((dispose) => dispose());
    diagnosticsDisposersRef.current = [];
  }, [stream.id]);

  useEffect(() => {
    if (blame && blame.path !== filePath) setBlame(null);
  }, [filePath, blame]);

  useEffect(() => {
    const wasDirty = prevDirtyRef.current;
    prevDirtyRef.current = isDirty;
    if (!blame || blame.path !== filePath) return;
    if (wasDirty && !isDirty) {
      void refreshBlame(filePath);
    }
  }, [isDirty, filePath, blame]);

  useEffect(() => {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco) return;
    if (blame) {
      editor.updateOptions({ lineNumbers: "off", lineDecorationsWidth: BLAME_WIDTH });
      setBlameLineHeight(editor.getOption(monaco.editor.EditorOption.lineHeight));
      setBlameScrollTop(editor.getScrollTop());
      const d = editor.onDidScrollChange((e: any) => setBlameScrollTop(e.scrollTop));
      return () => {
        d.dispose();
        editor.updateOptions({ lineNumbers: "on", lineDecorationsWidth: 10 });
      };
    }
    return undefined;
  }, [blame, monacoReady]);

  async function refreshBlame(path: string) {
    try {
      const lines = await gitBlame(streamRef.current.id, path);
      if (filePathRef.current !== path) return;
      if (lines.length === 0) {
        setBlame(null);
        setLspStatus("No git blame available");
        setTimeout(() => setLspStatus((s) => (s === "No git blame available" ? null : s)), 2500);
        return;
      }
      setBlame({ path, lines });
    } catch (err) {
      setLspStatus(`Blame failed: ${String(err)}`);
    }
  }

  function toggleBlame() {
    if (!filePath) return;
    if (blame && blame.path === filePath) {
      setBlame(null);
      return;
    }
    void refreshBlame(filePath);
  }

  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !navigationTarget || navigationTarget.path !== filePath) return;
    editor.focus();
    const position = { lineNumber: navigationTarget.line, column: navigationTarget.column };
    editor.setPosition(position);
    editor.revealPositionInCenter(position);
  }, [filePath, navigationTarget]);

  async function goToDefinition() {
    const editor = editorRef.current;
    const model = editor?.getModel();
    if (!editor || !model) return;
    const position = editor.getPosition();
    if (!position) return;
    const languageId = model.getLanguageId();
    if (!isLspCandidateLanguage(languageId)) {
      setLspStatus(`LSP not configured for ${languageId}`);
      return;
    }
    const client = ensureLspClient(streamRef.current, languageId);
    const result = await client.request<unknown>("textDocument/definition", {
      textDocument: { uri: model.uri.toString() },
      position: {
        line: position.lineNumber - 1,
        character: position.column - 1,
      },
    });
    const target = normalizeDefinitionTarget(streamRef.current, result);
    if (!target) return;
    if (target.path === filePathRef.current) {
      editor.focus();
      editor.setPosition({ lineNumber: target.line, column: target.column });
      editor.revealPositionInCenter({ lineNumber: target.line, column: target.column });
      return;
    }
    await onNavigateRef.current(target);
  }

  function ensureLspClient(currentStream: Stream, languageId: string): LspClient {
    let client = lspClientsRef.current.get(languageId);
    if (!client) {
      client = new LspClient(currentStream.id, languageId);
      diagnosticsDisposersRef.current.push(client.onDiagnostics((uri, diagnostics) => {
        const monaco = monacoRef.current;
        if (!monaco) return;
        const model = monaco.editor.getModel(monaco.Uri.parse(uri));
        if (!model) return;
        monaco.editor.setModelMarkers(model, markerOwnerRef.current, diagnostics.map((diagnostic) => ({
          severity: diagnosticSeverity(monaco, diagnostic.severity),
          message: diagnostic.message,
          source: diagnostic.source,
          startLineNumber: diagnostic.range.start.line + 1,
          startColumn: diagnostic.range.start.character + 1,
          endLineNumber: diagnostic.range.end.line + 1,
          endColumn: diagnostic.range.end.character + 1,
        })));
      }));
      diagnosticsDisposersRef.current.push(client.onStatus((message) => {
        const currentLanguage = languageForPath(filePathRef.current);
        if (!isLspCandidateLanguage(currentLanguage) || currentLanguage !== languageId) return;
        setLspStatus(message);
      }));
      lspClientsRef.current.set(languageId, client);
    }
    return client;
  }

  const contextMenuItems: MenuItem[] = [
    {
      id: "editor.save",
      label: "Save",
      shortcut: "Ctrl/Cmd+S",
      enabled: !!filePath && isDirty,
      run: () => onSave(),
    },
    {
      id: "editor.find",
      label: "Find",
      shortcut: "Ctrl/Cmd+F",
      enabled: !!filePath,
      run: () => editorRef.current?.getAction("actions.find")?.run(),
    },
    {
      id: "editor.go-to-definition",
      label: "Go to Definition",
      shortcut: "F12",
      enabled: !!filePath && isLspCandidateLanguage(languageForPath(filePath)),
      run: () => goToDefinition(),
    },
    {
      id: "editor.format-document",
      label: "Format Document",
      enabled: !!filePath,
      run: () => editorRef.current?.getAction("editor.action.formatDocument")?.run(),
    },
    {
      id: "editor.copy-path",
      label: "Copy Path",
      enabled: !!filePath,
      run: () => (filePath ? navigator.clipboard.writeText(filePath) : undefined),
    },
    {
      id: "editor.annotate-blame",
      label: blame && blame.path === filePath ? "Hide Git Blame" : "Annotate with Git Blame",
      enabled: !!filePath,
      run: () => toggleBlame(),
    },
  ];

  const showBlame = blame && blame.path === filePath;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* File tabs live in the parent CenterTabs bar now; this component
          just hosts the single Monaco editor that swaps models on filePath. */}
      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>
        <div ref={hostRef} style={{ width: "100%", height: "100%", minHeight: 0 }} />
        {showBlame ? (
          <BlameOverlay
            lines={blame!.lines}
            scrollTop={blameScrollTop}
            lineHeight={blameLineHeight}
            onClick={(sha) => {
              if (sha.replace(/0/g, "") === "") return;
              onRevealCommitRef.current?.(sha);
            }}
          />
        ) : null}
        {filePath && lspStatus ? <div style={lspStatusStyle}>{lspStatus}</div> : null}
        {!filePath ? (
          <div style={emptyStyle}>
            <div>Select a file from the sidebar to open it here.</div>
          </div>
        ) : null}
        {contextMenu ? (
          <ContextMenu
            items={contextMenuItems}
            position={contextMenu}
            onClose={() => setContextMenu(null)}
          />
        ) : null}
      </div>
    </div>
  );
}

const BLAME_WIDTH = 150;

function BlameOverlay({
  lines,
  scrollTop,
  lineHeight,
  onClick,
}: {
  lines: BlameLine[];
  scrollTop: number;
  lineHeight: number;
  onClick(sha: string): void;
}) {
  const now = Date.now() / 1000;
  return (
    <div
      style={{
        position: "absolute",
        top: 0,
        left: 0,
        width: BLAME_WIDTH,
        height: "100%",
        overflow: "hidden",
        fontFamily: "var(--mono, monospace)",
        fontSize: 11,
        userSelect: "none",
        zIndex: 3,
      }}
    >
      <div style={{ position: "absolute", top: -scrollTop, left: 0, right: 0 }}>
        {lines.map((line) => {
          const uncommitted = line.sha.replace(/0/g, "") === "";
          const ageDays = uncommitted ? 0 : Math.max(0, (now - line.authorTime) / 86400);
          const bg = uncommitted ? "rgba(70,70,70,0.35)" : blameColor(ageDays);
          const date = uncommitted ? "" : formatBlameDate(line.authorTime);
          const author = uncommitted ? "" : truncateAuthor(line.author);
          return (
            <div
              key={line.line}
              title={uncommitted ? "Uncommitted" : `${line.sha.slice(0, 8)} ${line.author} <${line.authorMail}>\n${line.summary}`}
              onClick={() => onClick(line.sha)}
              style={{
                height: lineHeight,
                lineHeight: `${lineHeight}px`,
                padding: "0 6px",
                background: bg,
                color: "var(--fg, #ddd)",
                borderRight: "1px solid var(--border, #333)",
                whiteSpace: "nowrap",
                overflow: "hidden",
                textOverflow: "ellipsis",
                cursor: uncommitted ? "default" : "pointer",
                boxSizing: "border-box",
              }}
            >
              {uncommitted ? "" : `${date}  ${author}`}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function blameColor(ageDays: number): string {
  // Younger commits = warmer/more saturated; older = cooler/darker.
  if (ageDays < 7) return "rgba(96, 165, 250, 0.55)";
  if (ageDays < 30) return "rgba(96, 165, 250, 0.40)";
  if (ageDays < 180) return "rgba(96, 165, 250, 0.28)";
  if (ageDays < 365) return "rgba(120, 140, 170, 0.22)";
  if (ageDays < 1095) return "rgba(120, 140, 170, 0.14)";
  return "rgba(120, 140, 170, 0.08)";
}

function formatBlameDate(epochSec: number): string {
  const d = new Date(epochSec * 1000);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function truncateAuthor(author: string): string {
  if (author.length <= 14) return author;
  return `${author.slice(0, 13)}…`;
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

function closeFindWidget(editor: any) {
  editor.getContribution("editor.contrib.findController")?.closeFindWidget?.();
}

const lspStatusStyle: CSSProperties = {
  position: "absolute",
  right: 12,
  bottom: 12,
  padding: "4px 8px",
  borderRadius: 4,
  border: "1px solid var(--border)",
  background: "rgba(23, 23, 23, 0.92)",
  color: "var(--muted)",
  fontSize: 12,
  pointerEvents: "none",
  zIndex: 5,
};

function registerGoToDefinitionAction(monaco: any, editor: any, run: () => Promise<void>) {
  editor.addAction({
    id: "newde.goToDefinition",
    label: "Go to Definition",
    keybindings: [monaco.KeyCode.F12],
    contextMenuGroupId: "navigation",
    run,
  });
}

function registerLspProviders(
  monaco: any,
  getClient: (languageId: string) => LspClient,
  streamRef: MutableRefObject<Stream>,
) {
  for (const languageId of ["typescript", "javascript"]) {
    monaco.languages.registerDefinitionProvider(languageId, {
      provideDefinition: async (model: any, position: any) => {
        const client = getClient(languageId);
        const result = await client.request<unknown>("textDocument/definition", {
          textDocument: { uri: model.uri.toString() },
          position: {
            line: position.lineNumber - 1,
            character: position.column - 1,
          },
        });
        return definitionResultToMonacoLocations(monaco, streamRef.current, result);
      },
    });
    monaco.languages.registerHoverProvider(languageId, {
      provideHover: async (model: any, position: any) => {
        const client = getClient(languageId);
        const result = await client.request<any>("textDocument/hover", {
          textDocument: { uri: model.uri.toString() },
          position: {
            line: position.lineNumber - 1,
            character: position.column - 1,
          },
        });
        if (!result?.contents) return null;
        return {
          contents: normalizeHoverContents(result.contents),
          range: result.range ? toMonacoRange(monaco, result.range) : undefined,
        };
      },
    });
    monaco.languages.registerReferenceProvider(languageId, {
      provideReferences: async (model: any, position: any) => {
        const client = getClient(languageId);
        const result = await client.request<unknown[]>("textDocument/references", {
          textDocument: { uri: model.uri.toString() },
          position: {
            line: position.lineNumber - 1,
            character: position.column - 1,
          },
          context: { includeDeclaration: true },
        });
        return Array.isArray(result)
          ? result
            .map((item) => referenceToMonacoLocation(monaco, item))
            .filter(Boolean)
          : [];
      },
    });
  }
}

function normalizeDefinitionTarget(stream: Stream, result: unknown): EditorNavigationTarget | null {
  const locations = Array.isArray(result) ? result : result ? [result] : [];
  for (const location of locations) {
    if (!location || typeof location !== "object") continue;
    const candidate = location as {
      uri?: string;
      targetUri?: string;
      range?: { start?: { line?: number; character?: number } };
      targetSelectionRange?: { start?: { line?: number; character?: number } };
      targetRange?: { start?: { line?: number; character?: number } };
    };
    if (candidate.targetUri) {
      const target = toEditorNavigationTarget(stream, candidate.targetUri, candidate.targetSelectionRange ?? candidate.targetRange);
      if (target) return target;
    }
    if (candidate.uri) {
      const target = toEditorNavigationTarget(stream, candidate.uri, candidate.range);
      if (target) return target;
    }
  }
  return null;
}

function definitionResultToMonacoLocations(monaco: any, stream: Stream, result: unknown): any[] {
  const locations = Array.isArray(result) ? result : result ? [result] : [];
  return locations
    .map((item) => referenceToMonacoLocation(monaco, item))
    .filter(Boolean);
}

function referenceToMonacoLocation(monaco: any, item: unknown): any | null {
  if (!item || typeof item !== "object") return null;
  const candidate = item as {
    uri?: string;
    targetUri?: string;
    range?: unknown;
    targetSelectionRange?: unknown;
    targetRange?: unknown;
  };
  const uri = candidate.targetUri ?? candidate.uri;
  const range = candidate.targetSelectionRange ?? candidate.targetRange ?? candidate.range;
  if (!uri || !range) return null;
  return {
    uri: monaco.Uri.parse(uri),
    range: toMonacoRange(monaco, range),
  };
}

function toMonacoRange(monaco: any, range: unknown): any {
  const candidate = range as {
    start?: { line?: number; character?: number };
    end?: { line?: number; character?: number };
  };
  return new monaco.Range(
    (candidate.start?.line ?? 0) + 1,
    (candidate.start?.character ?? 0) + 1,
    (candidate.end?.line ?? candidate.start?.line ?? 0) + 1,
    (candidate.end?.character ?? candidate.start?.character ?? 0) + 1,
  );
}

function normalizeHoverContents(contents: unknown): { value: string }[] {
  const values = Array.isArray(contents) ? contents : [contents];
  return values.flatMap((item) => {
    if (!item) return [];
    if (typeof item === "string") return [{ value: item }];
    if (typeof item === "object" && "value" in item && typeof (item as { value?: unknown }).value === "string") {
      return [{ value: (item as { value: string }).value }];
    }
    if (typeof item === "object" && "language" in item && "value" in item) {
      const markup = item as { language?: unknown; value?: unknown };
      if (typeof markup.value === "string") {
        return [{ value: `\`\`\`${typeof markup.language === "string" ? markup.language : ""}\n${markup.value}\n\`\`\`` }];
      }
    }
    return [];
  });
}

function diagnosticSeverity(monaco: any, severity?: number): number {
  switch (severity) {
    case 1:
      return monaco.MarkerSeverity.Error;
    case 2:
      return monaco.MarkerSeverity.Warning;
    case 3:
      return monaco.MarkerSeverity.Info;
    default:
      return monaco.MarkerSeverity.Hint;
  }
}
