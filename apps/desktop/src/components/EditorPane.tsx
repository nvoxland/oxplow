import type { CSSProperties, MutableRefObject } from "react";
import { useEffect, useRef, useState } from "react";
import type { BlameLine, Stream } from "../api.js";
import { desktopBridge, installLspPackage, readAt } from "../api.js";
import { computeDiffDecorations } from "../editor-diff.js";
import { languageForPath } from "../editor-language.js";
import { hasLspServer, refreshLspServers } from "../lsp-servers-store.js";
import type { EditorNavigationTarget } from "../lsp.js";
import { normalizeDefinitionTarget } from "../lsp-monaco-mapping.js";
import { getSuggestedLspPackage } from "../lspSuggestions.js";
import { logUi } from "../logger.js";
import { isDirty as bufferDirty, useBuffer } from "../editor/buffers.js";
import { buffers, loadMonaco } from "../editor/openFiles.js";
import { LspMirror } from "../editor/lspMirror.js";
import { BLAME_WIDTH, useBlame } from "./useBlame.js";
import { useMonacoViewState } from "./useMonacoViewState.js";
import type { MenuItem } from "../menu.js";
import { ContextMenu } from "./ContextMenu.js";
import { gitRevision } from "../revision.js";
import { selectionUri } from "../agent-context-ref.js";
import { addToContext } from "../contextBasket.js";
import { askAssistant } from "./Assistant/assistantSession.js";
import { MonacoCommentLayer, type MonacoCommentHandle } from "./Comments/MonacoCommentLayer.js";
import { getPageDetailStore } from "../tabs/openPageDetail.js";
import { fileRef } from "../tabs/pageRefs.js";
import { fileUri } from "../uri/uri.js";

interface Props {
  stream: Stream;
  /** The working tree's file it edits: its buffer (`editor/buffers.ts`),
   *  open and ready. */
  filePath: string;
  /** The line it lands on once the file shows (`#lines=`). */
  line: number | null;
  onSave(): void;
  /** Open another file at a line (Go to Definition). */
  onNavigateToLocation(target: EditorNavigationTarget): void;
  onRevealCommit?(sha: string): void;
  onCompareWithClipboard?(selection: string, path: string): void;
  /** Where it puts its Find, for the menu's. */
  findRef?: MutableRefObject<(() => void) | null>;
}

/** The most of a selection's text an editor publishes as its page detail. */
const PUBLISHED_SELECTION = 4_000;

export function EditorPane({
  stream,
  filePath,
  line,
  onSave,
  onNavigateToLocation,
  onRevealCommit,
  onCompareWithClipboard,
  findRef,
}: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  const editorRef = useRef<any>(null);
  const monacoRef = useRef<any>(null);
  const focusDisposersRef = useRef<{ dispose(): void }[]>([]);
  const focusDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const buffer = useBuffer(buffers, stream.id, filePath);
  const isDirty = bufferDirty(buffer);
  const ready = buffer?.state === "ready";
  const onSaveRef = useRef(onSave);
  useEffect(() => { onSaveRef.current = onSave; }, [onSave]);
  const onNavigateRef = useRef(onNavigateToLocation);
  const streamRef = useRef(stream);
  const filePathRef = useRef(filePath);
  // The status banner: this editor's own word (blame, Go to Definition),
  // else what the stream's language server says about this file's
  // language (the stream's LSP mirror, `editor/lspMirror.ts`).
  const [ownStatus, setLspStatus] = useState<string | null>(null);
  const [, heardMirror] = useState(0);
  useEffect(() => LspMirror.of(stream.id)?.subscribe(() => heardMirror((n) => n + 1)), [stream.id, ready]);
  const languageId = languageForPath(filePath);
  const mirrorStatus = LspMirror.of(stream.id)?.status(languageId) ?? null;
  const lspStatus = ownStatus ?? mirrorStatus;
  const lspInstallSuggestion = mirrorStatus && /LSP unavailable/i.test(mirrorStatus) ? getSuggestedLspPackage(languageId) : null;
  const [lspInstalling, setLspInstalling] = useState(false);
  async function installSuggested() {
    if (!lspInstallSuggestion) return;
    setLspInstalling(true);
    try {
      await installLspPackage(lspInstallSuggestion);
      setLspStatus(`Installed ${lspInstallSuggestion} — the server starts for the open files`);
      await refreshLspServers();
      LspMirror.of(stream.id)?.restart(languageId);
    } catch (err) {
      setLspStatus(`Install failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setLspInstalling(false);
    }
  }
  const clientFor = (lang: string) => {
    const mirror = LspMirror.of(streamRef.current.id);
    if (!mirror) throw new Error("this file isn't open");
    return mirror.client(lang);
  };
  const flushPendingChanges = (path: string | null) => {
    if (path) LspMirror.of(streamRef.current.id)?.flush(path);
  };
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);
  const [blameMenu, setBlameMenu] = useState<{ x: number; y: number; sha: string; authorMail: string } | null>(null);
  const onRevealCommitRef = useRef(onRevealCommit);
  onRevealCommitRef.current = onRevealCommit;
  const headByPathRef = useRef<Map<string, string | null>>(new Map());
  const diffDecoIdsRef = useRef<string[]>([]);
  const commentLayerRef = useRef<MonacoCommentHandle | null>(null);
  // Comment id under the last right-click (drives the "Open Comment"
  // menu item); set in onContextMenu before the menu opens.
  const ctxCommentIdRef = useRef<string | null>(null);
  // Monaco loads asynchronously, so the model-binding effect below needs a
  // signal to retry once it lands — otherwise the first file opened arrives
  // before the editor instance exists and the effect's early return makes
  // the pane stay blank until the next file change.
  const [monacoReady, setMonacoReady] = useState(false);

  // Blame overlay (merged local+git attribution + gutter scroll sync +
  // refresh-on-save) lives in useBlame; the overlay is rendered below.
  const { blame, blameScrollTop, blameLineHeight, toggleBlame } = useBlame({
    editorRef,
    monacoRef,
    monacoReady,
    streamRef,
    filePathRef,
    filePath,
    isDirty,
    setLspStatus,
  });

  onNavigateRef.current = onNavigateToLocation;
  streamRef.current = stream;
  filePathRef.current = filePath;

  // Monaco's view state (cursor / scroll / folds / selection) stays with the
  // tab's entry: Back and a restart land on the same line.
  const attachViewState = useMonacoViewState(`viewState:${filePath ?? ""}`);
  const detachViewStateRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      // Loaded once, the LSP providers registered on it for every file.
      const monaco = await loadMonaco();
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
      detachViewStateRef.current = attachViewState(editor);
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
        // Is the right-click on commented text? Drives "Open Comment".
        ctxCommentIdRef.current = position
          ? (commentLayerRef.current?.commentIdAt(position) ?? null)
          : null;
        const browserEvent = event.event?.browserEvent as MouseEvent | undefined;
        // Suppress the webview's native context menu (Reload / Inspect /
        // AutoFill) so only our custom menu shows — Monaco with
        // `contextmenu: false` doesn't preventDefault the DOM event.
        browserEvent?.preventDefault();
        setContextMenu({
          x: browserEvent?.clientX ?? 8,
          y: browserEvent?.clientY ?? 8,
        });
      });
      setMonacoReady(true);
    })();
    return () => {
      cancelled = true;
      focusDisposersRef.current.forEach((d) => d.dispose());
      focusDisposersRef.current = [];
      if (focusDebounceRef.current) clearTimeout(focusDebounceRef.current);
      detachViewStateRef.current?.();
      // The model is its buffer's (the store disposes it); the editor is ours.
      editorRef.current?.setModel(null);
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
    const openFilesPayload = buffers.list(currentStream.id).map((b) => ({ path: b.path, dirty: bufferDirty(b) }));
    void desktopBridge().updateEditorFocus({
      streamId: currentStream.id,
      activeFile: currentPath,
      caret,
      selection,
      openFiles: openFilesPayload,
    });
    // What an agent's `get_open_page` reads of this file's tab: the caret
    // or the selection (its text cut short).
    if (currentPath) {
      getPageDetailStore().publish(fileRef(currentPath).id, {
        path: currentPath,
        caret,
        selection: selection && {
          ...selection,
          text: selection.text.length > PUBLISHED_SELECTION ? `${selection.text.slice(0, PUBLISHED_SELECTION)}…` : selection.text,
        },
      });
    }
  }

  useEffect(() => {
    setContextMenu(null);
  }, [filePath, stream.id]);

  useEffect(() => {
    pushEditorFocus();
  }, [stream.id, filePath, isDirty]);

  // The editor shows its buffer's model: the draft, shared with every
  // editor of the file. Edits to it are the buffer's (the store listens),
  // and the stream's LSP mirror sends them on.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !ready) return;
    const model = buffers.model(stream.id, filePath);
    if (!model || editor.getModel() === model) return;
    closeFindWidget(editor);
    editor.setModel(model);
  }, [stream.id, filePath, ready, monacoReady]);

  // The menu's Find (Ctrl/Cmd+F outside the editor).
  useEffect(() => {
    if (!findRef) return;
    findRef.current = () => {
      editorRef.current?.focus();
      void editorRef.current?.getAction("actions.find")?.run();
    };
    return () => {
      findRef.current = null;
    };
  }, [findRef]);

  useEffect(() => {
    if (!filePath) return;
    let cancelled = false;
    const sid = stream.id;
    const t0 = performance.now();
    logUi("debug", "editor: readHEAD start", { path: filePath });
    (async () => {
      try {
        const content = await readAt(sid, filePath, gitRevision("HEAD"));
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
  }, [buffer?.draft, filePath, monacoReady, ready]);

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

  // Land on `line` (the page's `#lines=`) once the file shows.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || line === null || !ready || !monacoReady) return;
    editor.focus();
    const position = { lineNumber: line, column: 1 };
    editor.setPosition(position);
    editor.revealPositionInCenter(position);
  }, [filePath, line, ready, monacoReady]);

  async function goToDefinition() {
    const editor = editorRef.current;
    const model = editor?.getModel();
    if (!editor || !model) return;
    const position = editor.getPosition();
    if (!position) return;
    const lang = model.getLanguageId();
    if (!hasLspServer(lang)) {
      setLspStatus(`LSP not configured for ${lang}`);
      return;
    }
    flushPendingChanges(filePathRef.current);
    const client = clientFor(lang);
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
    onNavigateRef.current(target);
  }

  const contextMenuItems: MenuItem[] = [
    {
      id: "editor.cut",
      label: "Cut",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => {
        const ed = editorRef.current;
        const model = ed?.getModel();
        const sel = ed?.getSelection?.();
        if (!ed || !model || !sel || sel.isEmpty()) return;
        void navigator.clipboard.writeText(model.getValueInRange(sel));
        ed.executeEdits("cut", [{ range: sel, text: "" }]);
        ed.focus();
      },
    },
    {
      id: "editor.copy",
      label: "Copy",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => {
        const ed = editorRef.current;
        const model = ed?.getModel();
        const sel = ed?.getSelection?.();
        if (!ed || !model || !sel || sel.isEmpty()) return;
        void navigator.clipboard.writeText(model.getValueInRange(sel));
      },
    },
    {
      id: "editor.paste",
      label: "Paste",
      enabled: !!filePath,
      run: async () => {
        const ed = editorRef.current;
        const sel = ed?.getSelection?.();
        if (!ed || !sel) return;
        const t = await navigator.clipboard.readText();
        if (t) {
          ed.executeEdits("paste", [{ range: sel, text: t }]);
          ed.focus();
        }
      },
    },
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
      enabled: !!filePath && hasLspServer(languageForPath(filePath)),
      run: () => goToDefinition(),
    },
    {
      id: "editor.rename-symbol",
      label: "Rename Symbol",
      shortcut: "F2",
      enabled: !!filePath && hasLspServer(languageForPath(filePath)),
      run: () => editorRef.current?.getAction("editor.action.rename")?.run(),
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
    {
      id: "editor.ask-about",
      label: "Ask the Assistant",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => {
        const editor = editorRef.current;
        const sel = editor?.getSelection?.();
        const model = editor?.getModel();
        if (!editor || !model || !sel || sel.isEmpty() || !filePath) return;
        void askAssistant({ kind: "selection", uri: selectionUri(filePath, sel), text: model.getValueInRange(sel), label: null });
      },
    },
    {
      id: "editor.add-to-context",
      label: "Add Selection to Context",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => {
        const editor = editorRef.current;
        const sel = editor?.getSelection?.();
        const model = editor?.getModel();
        if (!editor || !model || !sel || sel.isEmpty() || !filePath) return;
        void addToContext({ uri: selectionUri(filePath, sel), text: model.getValueInRange(sel) });
      },
    },
    {
      id: "editor.add-comment",
      label: "Add Comment",
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => commentLayerRef.current?.addCommentForSelection(),
    },
    {
      id: "editor.open-comment",
      label: "Open Comment",
      enabled: ctxCommentIdRef.current != null,
      run: () => {
        const id = ctxCommentIdRef.current;
        if (id != null) commentLayerRef.current?.openComment(id);
      },
    },
    // Re-attach orphaned comments to the current selection — the escape
    // hatch when a quote drifted past fuzzy tolerance.
    ...(commentLayerRef.current?.relinkTargets() ?? []).map((t) => ({
      id: `editor.relink-${t.id}`,
      label: `Relink orphaned: “${t.quote.length > 24 ? `${t.quote.slice(0, 24)}…` : t.quote}”`,
      enabled: !!filePath && hasEditorSelection(editorRef.current),
      run: () => commentLayerRef.current?.relinkToSelection(t.id),
    })),
  ];

  const showBlame = blame && blame.path === filePath;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      {/* File tabs live in the parent CenterTabs bar now; this component
          just hosts the single Monaco editor that swaps models on filePath. */}
      <div
        style={{ position: "relative", flex: 1, minHeight: 0, width: "100%" }}
        // Belt-and-suspenders: kill the webview's native context menu in
        // the editor area so only our custom menu shows (Monaco's own
        // onContextMenu preventDefault can miss depending on timing).
        onContextMenu={(e) => e.preventDefault()}
      >
        <div ref={hostRef} data-testid="monaco-host" data-file-path={filePath ?? ""} style={{ width: "100%", height: "100%", minHeight: 0 }} />
        {monacoReady && ready ? (
          <MonacoCommentLayer
            ref={commentLayerRef}
            editor={editorRef.current}
            monaco={monacoRef.current}
            ready={monacoReady}
            streamId={stream.id}
            threadId={null}
            target={fileUri(filePath)}
          />
        ) : null}
        {showBlame ? (
          <BlameOverlay
            entries={blame!.entries}
            scrollTop={blameScrollTop}
            lineHeight={blameLineHeight}
            onGitClick={(sha) => {
              onRevealCommitRef.current?.(sha);
            }}
            onOpenGitMenu={(rect, sha, authorMail) => {
              setBlameMenu({ x: rect.right, y: rect.bottom + 4, sha, authorMail });
            }}
          />
        ) : null}
        {lspStatus ? (
          <div style={lspStatusStyle}>
            <span>{lspStatus}</span>
            {lspInstallSuggestion ? (
              <button
                type="button"
                disabled={lspInstalling}
                onClick={() => void installSuggested()}
                style={lspInstallButtonStyle}
              >
                {lspInstalling ? `Installing ${lspInstallSuggestion}…` : `Install ${lspInstallSuggestion}`}
              </button>
            ) : null}
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

function BlameOverlay({
  entries,
  scrollTop,
  lineHeight,
  onGitClick,
  onOpenGitMenu,
}: {
  entries: BlameLine[];
  scrollTop: number;
  lineHeight: number;
  onGitClick(sha: string): void;
  /**
   * Open the git-blame menu for `sha`. `rect` anchors the popover; a
   * right-click passes a zero-size rect at the cursor so the menu opens
   * where the user clicked. Per-row actions live on the right-click menu
   * (see `.context/usability.md`), not a kebab button.
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
        fontFamily: "var(--font-mono)",
        fontSize: 11,
        userSelect: "none",
        zIndex: 3,
      }}
    >
      <div style={{ position: "absolute", top: -scrollTop, left: 0, right: 0 }}>
        {entries.map((entry) => {
          if (entry.revision) {
            const ageDays = Math.max(0, (nowSec - entry.time) / 86400);
            const bg = blameGitColor(ageDays);
            const date = formatBlameDate(entry.time);
            const author = truncateAuthor(entry.author);
            const sha = entry.revision;
            const authorMail = entry.email;
            return (
              <div
                key={entry.line}
                title={`${sha.slice(0, 8)} ${entry.author} <${authorMail}>\n${entry.summary}`}
                onClick={() => onGitClick(sha)}
                onContextMenu={(event) => {
                  event.preventDefault();
                  onOpenGitMenu(new DOMRect(event.clientX, event.clientY, 0, 0), sha, authorMail);
                }}
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
  zIndex: 5,
  display: "flex",
  alignItems: "center",
  gap: 8,
};

const lspInstallButtonStyle: CSSProperties = {
  fontSize: 11,
  padding: "2px 6px",
  borderRadius: 3,
  border: "1px solid var(--border)",
  background: "var(--surface-card)",
  color: "var(--text-primary)",
  cursor: "pointer",
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
