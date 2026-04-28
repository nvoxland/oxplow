import type { CSSProperties } from "react";
import { useEffect, useRef, useState } from "react";
import type { OpenFileState } from "../../session/file-session.js";
import type { LocalBlameEntry, Stream } from "../api.js";
import { localBlame, readFileAtRef } from "../api.js";
import { isLspCandidateLanguage, languageForPath } from "../editor-language.js";
import { LspClient, type EditorNavigationTarget, streamFileUri, toEditorNavigationTarget } from "../lsp.js";
import { logUi } from "../logger.js";
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
  onRevealCommit?(sha: string): void;
  onRevealWorkItem?(itemId: string): void;
  onCompareWithClipboard?(selection: string, path: string): void;
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
  onRevealCommit,
  onRevealWorkItem,
  onCompareWithClipboard,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const changeDisposeRef = useRef<{ dispose(): void } | null>(null);
  const focusDisposersRef = useRef<{ dispose(): void }[]>([]);
  const focusDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const openFilesRef = useRef({ order: openFileOrder, files: openFiles });
  const onChangeRef = useRef(onChange);
  const onSaveRef = useRef(onSave);
  useEffect(() => { onSaveRef.current = onSave; }, [onSave]);
  const onNavigateRef = useRef(onNavigateToLocation);
  const streamRef = useRef(stream);
  const filePathRef = useRef(filePath);
  const lspClientsRef = useRef(new Map<string, LspClient>());
  const trackedOpenDocsRef = useRef(new Map<string, string>());
  const trackedSavedContentRef = useRef(new Map<string, string>());
  const diagnosticsDisposersRef = useRef<(() => void)[]>([]);
  const markerOwnerRef = useRef(`oxplow-lsp-${stream.id}`);
  const [lspStatus, setLspStatus] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [blameMenu, setBlameMenu] = useState<{ x: number; y: number; sha: string; authorMail: string } | null>(null);
  const [blame, setBlame] = useState<{ path: string; entries: LocalBlameEntry[] } | null>(null);
  const [blameScrollTop, setBlameScrollTop] = useState(0);
  const [blameLineHeight, setBlameLineHeight] = useState(19);
  const prevDirtyRef = useRef(isDirty);
  const onRevealCommitRef = useRef(onRevealCommit);
  onRevealCommitRef.current = onRevealCommit;
  const onRevealWorkItemRef = useRef(onRevealWorkItem);
  onRevealWorkItemRef.current = onRevealWorkItem;
  const headByPathRef = useRef<Map<string, string | null>>(new Map());
  const diffDecoIdsRef = useRef<string[]>([]);
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
      // Register Cmd/Ctrl+S inside Monaco so the shortcut works when the
      // editor has focus. The native Electron menu also binds this — menu
      // accelerators fire at the OS level BEFORE the keydown reaches the
      // webview, so the two don't double-fire in practice. This path is
      // what makes the shortcut work under Playwright (synthetic keystrokes
      // bypass the native menu) and if the native menu is ever dismissed.
      editor.addCommand(
        monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS,
        () => { void onSaveRef.current(); },
      );
      registerGoToDefinitionAction(monaco, editor, () => goToDefinition());
      registerLspProviders(monaco, (languageId) => ensureLspClient(streamRef.current, languageId));
      focusDisposersRef.current.push(editor.onDidChangeCursorSelection(() => scheduleFocusPush()));
      focusDisposersRef.current.push(editor.onDidChangeCursorPosition(() => scheduleFocusPush()));
      editor.onContextMenu((event: any) => {
        if (!filePathRef.current) return;
        const position = event.target?.position ?? editor.getPosition();
        // Preserve any active selection so actions like "Compare with
        // Clipboard" can still see it; only move the caret when the
        // right-click landed outside the current selection.
        const currentSelection = editor.getSelection?.();
        const clickInsideSelection = position
          && currentSelection
          && !currentSelection.isEmpty()
          && currentSelection.containsPosition(position);
        if (position && !clickInsideSelection) {
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
    const api = (window as any).oxplowApi as { updateEditorFocus?: (payload: unknown) => unknown } | undefined;
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
    const t0 = performance.now();
    logUi("debug", "editor: model-setup start", { path: filePath, valueLength: value.length });

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
    logUi("debug", "editor: model-setup end", { path: filePath, ms: Math.round(performance.now() - t0) });
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
    const t0 = performance.now();
    logUi("debug", "editor: lsp-sync start", { fileCount: openFileOrder.length });
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
    logUi("debug", "editor: lsp-sync end", {
      fileCount: openFileOrder.length,
      tracked: trackedOpenDocsRef.current.size,
      ms: Math.round(performance.now() - t0),
    });
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
    markerOwnerRef.current = `oxplow-lsp-${stream.id}`;
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

  useEffect(() => {
    if (!filePath) return;
    let cancelled = false;
    const sid = stream.id;
    const t0 = performance.now();
    logUi("debug", "editor: readHEAD start", { path: filePath });
    (async () => {
      try {
        const { content } = await readFileAtRef(sid, "HEAD", filePath);
        if (cancelled) return;
        logUi("debug", "editor: readHEAD end", {
          path: filePath,
          headSize: content?.length ?? 0,
          ms: Math.round(performance.now() - t0),
        });
        headByPathRef.current.set(filePath, content);
        applyDiffDecorations();
      } catch {
        if (cancelled) return;
        logUi("debug", "editor: readHEAD end (error)", {
          path: filePath,
          ms: Math.round(performance.now() - t0),
        });
        headByPathRef.current.set(filePath, null);
        applyDiffDecorations();
      }
    })();
    return () => { cancelled = true; };
  }, [stream.id, filePath]);

  useEffect(() => {
    applyDiffDecorations();
  }, [value, filePath, monacoReady]);

  function applyDiffDecorations() {
    const editor = editorRef.current;
    const monaco = monacoRef.current;
    if (!editor || !monaco || !filePath) return;
    const model = editor.getModel();
    if (!model) return;
    const head = headByPathRef.current.get(filePath);
    if (head === undefined) {
      // HEAD not fetched yet — leave any existing decorations in place.
      return;
    }
    const t0 = performance.now();
    const newLines = model.getValue().split("\n");
    const oldLines = head === null ? [] : head.split("\n");
    logUi("debug", "editor: diff-decorations start", {
      path: filePath,
      oldLines: oldLines.length,
      newLines: newLines.length,
    });
    const decos = computeDiffDecorations(monaco, oldLines, newLines);
    diffDecoIdsRef.current = editor.deltaDecorations(diffDecoIdsRef.current, decos);
    logUi("debug", "editor: diff-decorations end", {
      path: filePath,
      decoCount: decos.length,
      ms: Math.round(performance.now() - t0),
    });
  }

  async function refreshBlame(path: string) {
    try {
      const entries = await localBlame(streamRef.current.id, path);
      if (filePathRef.current !== path) return;
      if (entries.length === 0) {
        setBlame(null);
        setLspStatus("No blame available");
        setTimeout(() => setLspStatus((s) => (s === "No blame available" ? null : s)), 2500);
        return;
      }
      setBlame({ path, entries });
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
      label: blame && blame.path === filePath ? "Hide Blame" : "Annotate with Blame",
      enabled: !!filePath,
      run: () => toggleBlame(),
    },
    {
      id: "editor.compare-clipboard",
      label: "Compare with Clipboard",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => {
        const editor = editorRef.current;
        const model = editor?.getModel();
        const sel = editor?.getSelection?.();
        if (!editor || !model || !sel || sel.isEmpty() || !filePath) return;
        const text = model.getValueInRange(sel);
        onCompareWithClipboard?.(text, filePath);
      },
    },
  ];

  const showBlame = blame && blame.path === filePath;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* File tabs live in the parent CenterTabs bar now; this component
          just hosts the single Monaco editor that swaps models on filePath. */}
      <div style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}>
        <div ref={hostRef} data-testid="monaco-host" data-file-path={filePath ?? ""} style={{ width: "100%", height: "100%", minHeight: 0 }} />
        {showBlame ? (
          <BlameOverlay
            entries={blame!.entries}
            scrollTop={blameScrollTop}
            lineHeight={blameLineHeight}
            onLocalClick={(itemId) => {
              onRevealWorkItemRef.current?.(itemId);
            }}
            onGitClick={(sha) => {
              onRevealCommitRef.current?.(sha);
            }}
            onOpenGitMenu={(rect, sha, authorMail) => {
              setBlameMenu({ x: rect.right, y: rect.bottom + 4, sha, authorMail });
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
        {blameMenu ? (
          <ContextMenu
            items={[
              {
                id: "blame.copy-sha",
                label: "Copy commit SHA",
                enabled: true,
                run: () => { void navigator.clipboard?.writeText(blameMenu.sha); },
              },
              {
                id: "blame.reveal",
                label: "Reveal commit",
                enabled: true,
                run: () => { onRevealCommitRef.current?.(blameMenu.sha); },
              },
              {
                id: "blame.copy-author-email",
                label: "Copy author email",
                enabled: !!blameMenu.authorMail,
                run: () => { void navigator.clipboard?.writeText(blameMenu.authorMail); },
              },
            ]}
            position={{ x: blameMenu.x, y: blameMenu.y }}
            onClose={() => setBlameMenu(null)}
          />
        ) : null}
      </div>
    </div>
  );
}

function hasEditorSelection(editor: any): boolean {
  const sel = editor?.getSelection?.();
  return !!sel && !sel.isEmpty();
}

// Line-level diff between old and new arrays. Returns, for each line in
// `newLines` (1-indexed), either "added", "modified", or null. "modified"
// is an added line that sits next to a deletion — i.e. a changed line.
// "deleted" regions are collapsed onto the next surviving line as a
// bottom marker.
function diffLineKinds(oldLines: string[], newLines: string[]): {
  kinds: Array<null | "added" | "modified">;
  deletedBefore: boolean[]; // index N means: a pure deletion happened just before newLines[N]
} {
  const m = oldLines.length, n = newLines.length;
  // Guard: very large files — skip diffing to avoid quadratic blowup.
  if (m > 5000 || n > 5000) {
    return { kinds: new Array(n).fill(null), deletedBefore: new Array(n + 1).fill(false) };
  }
  const dp: Int32Array[] = new Array(m + 1);
  for (let i = 0; i <= m; i++) dp[i] = new Int32Array(n + 1);
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] = oldLines[i - 1] === newLines[j - 1]
        ? dp[i - 1][j - 1] + 1
        : Math.max(dp[i - 1][j], dp[i][j - 1]);
    }
  }
  const added = new Array<boolean>(n).fill(false);
  const deleted = new Array<boolean>(m).fill(false);
  let i = m, j = n;
  while (i > 0 && j > 0) {
    if (oldLines[i - 1] === newLines[j - 1]) { i--; j--; }
    else if (dp[i - 1][j] >= dp[i][j - 1]) { deleted[i - 1] = true; i--; }
    else { added[j - 1] = true; j--; }
  }
  while (i > 0) { deleted[i - 1] = true; i--; }
  while (j > 0) { added[j - 1] = true; j--; }

  // Walk the edit script in forward order to classify adds as
  // "modified" (accompanied by deletes in the same hunk) vs pure "added",
  // and to locate pure-deletion hunks (boundary markers).
  const kinds: Array<null | "added" | "modified"> = new Array(n).fill(null);
  const deletedBefore = new Array<boolean>(n + 1).fill(false);
  let oi = 0, nj = 0;
  while (oi < m || nj < n) {
    if (oi < m && nj < n && oldLines[oi] === newLines[nj]) { oi++; nj++; continue; }
    let addedCount = 0, deletedCount = 0;
    const hunkStart = nj;
    while (nj < n && added[nj]) { addedCount++; nj++; }
    while (oi < m && deleted[oi]) { deletedCount++; oi++; }
    if (addedCount > 0) {
      const kind = deletedCount > 0 ? "modified" : "added";
      for (let k = hunkStart; k < nj; k++) kinds[k] = kind;
    } else if (deletedCount > 0) {
      // Pure deletion — mark the boundary on the surviving line.
      deletedBefore[hunkStart] = true;
    }
  }
  return { kinds, deletedBefore };
}

function computeDiffDecorations(monaco: any, oldLines: string[], newLines: string[]): any[] {
  const { kinds, deletedBefore } = diffLineKinds(oldLines, newLines);
  const decos: any[] = [];
  for (let k = 0; k < kinds.length; k++) {
    const kind = kinds[k];
    if (!kind) continue;
    const line = k + 1;
    decos.push({
      range: new monaco.Range(line, 1, line, 1),
      options: {
        isWholeLine: true,
        linesDecorationsClassName: kind === "added" ? "oxplow-gutter-added" : "oxplow-gutter-modified",
      },
    });
  }
  // Render pure-deletion markers as a red bottom-bar on the line above
  // the missing content (or on line 1 if at the start of file).
  for (let k = 0; k < deletedBefore.length; k++) {
    if (!deletedBefore[k]) continue;
    const line = Math.max(1, Math.min(newLines.length, k));
    decos.push({
      range: new monaco.Range(line, 1, line, 1),
      options: {
        isWholeLine: true,
        linesDecorationsClassName: "oxplow-gutter-deleted",
      },
    });
  }
  return decos;
}

const BLAME_WIDTH = 150;

function BlameOverlay({
  entries,
  scrollTop,
  lineHeight,
  onLocalClick,
  onGitClick,
  onOpenGitMenu,
}: {
  entries: LocalBlameEntry[];
  scrollTop: number;
  lineHeight: number;
  onLocalClick(itemId: string): void;
  onGitClick(sha: string): void;
  /**
   * Open the git-blame menu for `sha`. `rect` is the bounding rect of the
   * trigger button so the popover can be anchored under it (matches the
   * Kebab/ContextMenu positioning convention used elsewhere). Replaces
   * the previous mouse-event-based right-click menu.
   */
  onOpenGitMenu(rect: DOMRect, sha: string, authorMail: string): void;
}) {
  const nowSec = Date.now() / 1000;
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
        {entries.map((entry) => {
          if (entry.source === "local" && entry.workItem) {
            const endedAtSec = Date.parse(entry.workItem.endedAt) / 1000;
            const ageDays = Math.max(0, (nowSec - endedAtSec) / 86400);
            const bg = blameLocalColor(ageDays);
            const label = truncateAuthor(entry.workItem.title);
            const itemId = entry.workItem.id;
            return (
              <div
                key={entry.line}
                title={`${entry.workItem.title}\nwork item ${itemId}\nfinished ${formatBlameDate(endedAtSec)}`}
                onClick={() => onLocalClick(itemId)}
                style={{
                  ...rowStyle(lineHeight, bg),
                  borderLeft: "2px solid var(--blame-local-border, #e5a06a)",
                  cursor: "pointer",
                }}
              >
                {label}
              </div>
            );
          }
          if (entry.source === "git" && entry.git) {
            const ageDays = Math.max(0, (nowSec - entry.git.authorTime) / 86400);
            const bg = blameGitColor(ageDays);
            const date = formatBlameDate(entry.git.authorTime);
            const author = truncateAuthor(entry.git.author);
            const sha = entry.git.sha;
            const authorMail = entry.git.authorMail;
            return (
              <div
                key={entry.line}
                title={`${sha.slice(0, 8)} ${entry.git.author} <${authorMail}>\n${entry.git.summary}`}
                onClick={() => onGitClick(sha)}
                className="oxplow-blame-row"
                style={{
                  ...rowStyle(lineHeight, bg),
                  borderLeft: "2px solid var(--blame-git-border, #4a9eff)",
                  cursor: "pointer",
                  display: "flex",
                  alignItems: "center",
                  gap: 4,
                  position: "relative",
                }}
              >
                <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {`${date}  ${author}`}
                </span>
                <button
                  type="button"
                  aria-label="Blame actions"
                  className="oxplow-blame-row-kebab"
                  onClick={(event) => {
                    event.stopPropagation();
                    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
                    onOpenGitMenu(rect, sha, authorMail);
                  }}
                  style={{
                    background: "transparent",
                    border: "none",
                    color: "var(--muted)",
                    cursor: "pointer",
                    fontSize: 11,
                    lineHeight: 1,
                    padding: "0 2px",
                  }}
                >
                  ⋯
                </button>
              </div>
            );
          }
          // uncommitted
          return (
            <div
              key={entry.line}
              title="Uncommitted"
              style={{
                ...rowStyle(lineHeight, "var(--blame-uncommitted, rgba(70,70,70,0.35))"),
                borderLeft: "2px solid transparent",
                cursor: "default",
              }}
            />
          );
        })}
      </div>
    </div>
  );
}

function rowStyle(lineHeight: number, background: string): CSSProperties {
  return {
    height: lineHeight,
    lineHeight: `${lineHeight}px`,
    padding: "0 6px",
    background,
    color: "var(--fg, #ddd)",
    borderRight: "1px solid var(--border, #333)",
    whiteSpace: "nowrap",
    overflow: "hidden",
    textOverflow: "ellipsis",
    boxSizing: "border-box",
  };
}

function blameLocalColor(ageDays: number): string {
  if (ageDays < 7) return "var(--blame-local-fresh, rgba(229,160,106,0.55))";
  if (ageDays < 30) return "var(--blame-local-recent, rgba(229,160,106,0.40))";
  if (ageDays < 180) return "var(--blame-local-stale, rgba(229,160,106,0.28))";
  return "var(--blame-local-old, rgba(170,140,110,0.18))";
}

function blameGitColor(ageDays: number): string {
  if (ageDays < 7) return "var(--blame-git-fresh, rgba(96,165,250,0.55))";
  if (ageDays < 30) return "var(--blame-git-recent, rgba(96,165,250,0.40))";
  if (ageDays < 180) return "var(--blame-git-stale, rgba(96,165,250,0.28))";
  return "var(--blame-git-old, rgba(120,140,170,0.14))";
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
    id: "oxplow.goToDefinition",
    label: "Go to Definition",
    keybindings: [monaco.KeyCode.F12],
    contextMenuGroupId: "navigation",
    run,
  });
}

function registerLspProviders(
  monaco: any,
  getClient: (languageId: string) => LspClient,
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
        return definitionResultToMonacoLocations(monaco, result);
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

function definitionResultToMonacoLocations(monaco: any, result: unknown): any[] {
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
