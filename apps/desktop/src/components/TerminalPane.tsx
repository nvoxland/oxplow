import { useEffect, useRef, useState, type DragEvent as ReactDragEvent } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import type { TerminalEvent } from "../editor-session.js";
import { desktopBridge } from "../api.js";
import { logUi } from "../logger.js";
import {
  shouldHandleTerminalPageKey,
  shouldReturnTerminalToPrompt,
  shouldRouteWheelToTmuxHistory,
  wheelDeltaToScrollLines,
} from "../terminal-scroll.js";
import { subscribeAgentInput } from "../agent-input-bus.js";
import { TerminalCommentLayer } from "./Comments/TerminalCommentLayer.js";
import {
  WORK_ITEM_DRAG_MIME_VALUE,
  decodeTaskDragRefs,
  dragHasContextRef,
  dragHasTaskRefs,
  readContextRef,
} from "../agent-context-dnd.js";
import { formatContextMention } from "../agent-context-ref.js";
import { installFilePathLinkProvider, type FilePathLinkActivation } from "../terminal-link-provider.js";

const XTERM_THEME = {
  background: "#1c1f24",
  foreground: "#e6e8ec",
  cursor: "#5b8cf5",
  cursorAccent: "#1c1f24",
  selectionBackground: "#1f2a44",
  black: "#3f4451",
  red: "#e05561",
  green: "#8cc265",
  yellow: "#d18f52",
  blue: "#4aa5f0",
  magenta: "#c162de",
  cyan: "#42b3c2",
  white: "#e6e6e6",
  brightBlack: "#4f5666",
  brightRed: "#ff616e",
  brightGreen: "#a5e075",
  brightYellow: "#f0a45d",
  brightBlue: "#4dc4ff",
  brightMagenta: "#de73ff",
  brightCyan: "#4cd1e0",
  brightWhite: "#ffffff",
} as const;

/**
 * Read the system clipboard as text. Prefers Electron's main-process
 * clipboard (via IPC) because navigator.clipboard.readText() in the
 * renderer rejects with "Document is not focused" on a fast Cmd-Tab →
 * Cmd+V and returns empty for non-text-primary flavors set by other
 * apps. Falls back to navigator.clipboard if the IPC path isn't wired.
 */
/**
 * Resolve a path detected in terminal output against the active
 * stream's worktree. Absolute paths are returned as-is; `~/...` is
 * left unresolved (frontend doesn't know HOME) and dropped; relative
 * paths join onto the worktree. Returns null when the path can't be
 * resolved (no worktree + relative path).
 */
function resolveAgainstWorktree(path: string, worktree: string | undefined): string | null {
  if (!path) return null;
  if (path.startsWith("/")) return path;
  if (path.startsWith("~/")) return null;
  if (!worktree) return null;
  const trimmed = worktree.endsWith("/") ? worktree.slice(0, -1) : worktree;
  // Strip explicit ./ to keep the joined path tidy.
  const rel = path.startsWith("./") ? path.slice(2) : path;
  return `${trimmed}/${rel}`;
}

/// Resolve a clicked terminal path to an absolute path for the editor.
/// Absolute paths pass through; `~/` is dropped (frontend doesn't know HOME).
/// A relative path is resolved against the session's *live* cwd — so a path
/// printed after `cd`ing into a subdir opens correctly — falling back to the
/// worktree root when the cwd can't be determined (tmux pane, dead session,
/// unsupported platform).
async function resolveClickedPath(
  text: string,
  sessionId: string | null,
  worktree: string | undefined,
): Promise<string | null> {
  if (!text) return null;
  if (text.startsWith("/")) return text;
  if (text.startsWith("~/")) return null;
  let base = worktree;
  if (sessionId) {
    const cwd = await desktopBridge().terminalSessionCwd(sessionId);
    if (cwd) base = cwd;
  }
  return resolveAgainstWorktree(text, base);
}

async function readClipboard(): Promise<string> {
  const api = desktopBridge() as { clipboardReadText?: () => Promise<string> };
  if (api?.clipboardReadText) {
    try {
      return await api.clipboardReadText();
    } catch {
      // fall through to navigator.clipboard
    }
  }
  try {
    return await navigator.clipboard.readText();
  } catch {
    return "";
  }
}

export function TerminalPane({
  paneTarget,
  visible,
  transportMode,
  onUserInterrupt,
  worktreePath,
  onOpenFile,
  comments,
}: {
  paneTarget: string;
  visible: boolean;
  transportMode: "direct" | "tmux";
  /// Fires when the user presses Escape in live mode (i.e. signals
  /// Claude Code to cancel the in-flight turn). Lets the host
  /// synthesize an Interrupt hook so the working-dot flips to idle
  /// even though Claude Code itself doesn't emit a Stop. */
  onUserInterrupt?(): void;
  /// Stream's worktree absolute path. Relative paths detected in
  /// terminal output resolve against this. Optional — when omitted,
  /// only absolute paths produce links.
  worktreePath?: string;
  /// Fires when the user clicks a detected file-path link. The
  /// callback receives an absolute path (resolved against
  /// `worktreePath` when the source was relative) plus optional
  /// line/column.
  onOpenFile?(absPath: string, line?: number, column?: number): void;
  /// When set, the pane becomes comment-enabled: a selection on terminal
  /// output can be commented, anchored to the buffer text and targeted at
  /// this page ref. `threadId` is null for the shell terminal.
  comments?: { streamId: string; threadId: string | null; targetKind: string; targetId: string };
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  // Mirror of `termRef.current` as state so the comment layer mounts once
  // the terminal is opened (and unmounts on dispose).
  const [term, setTerm] = useState<Terminal | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const [mode, setMode] = useState<"live" | "history">("live");
  const modeRef = useRef<"live" | "history">("live");
  const [dragHovering, setDragHovering] = useState(false);
  // Live refs so the link-provider activate handler always sees the
  // current worktree + callback even though the provider is
  // registered once at terminal creation.
  const worktreePathRef = useRef<string | undefined>(worktreePath);
  const onOpenFileRef = useRef<typeof onOpenFile>(onOpenFile);
  worktreePathRef.current = worktreePath;
  onOpenFileRef.current = onOpenFile;

  function setInteractionMode(next: "live" | "history") {
    modeRef.current = next;
    setMode(next);
  }

  useEffect(() => {
    if (!visible) return;
    termRef.current?.focus();
    if (transportMode === "tmux" && sessionIdRef.current) {
      void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
    }
    setInteractionMode("live");
  }, [paneTarget, transportMode, visible]);

  // Subscribe to the "Add to agent context" bus only while this pane is
  // visible — `insertIntoAgent` from a drag-drop or right-click anywhere
  // in the UI naturally targets the agent the user is currently looking
  // at. `term.paste(text)` writes through xterm's input pipeline so the
  // existing `onData` handler ships the bytes to the agent process for
  // both direct and tmux transports — no transport branching here.
  useEffect(() => {
    if (!visible) return;
    const unsub = subscribeAgentInput((text) => {
      const term = termRef.current;
      if (!term) return;
      term.paste(text);
      term.focus();
    });
    return unsub;
  }, [visible]);

  function handleDragOver(e: ReactDragEvent<HTMLDivElement>) {
    // Accept either the standalone "context ref" MIME (file/note/tasks
    // single-row drag) or a multi-id tasks DnD payload (Plan pane
    // marked-set drag). Both end up inserted as @-mentions / bracketed
    // refs through the same `term.paste` pipeline.
    if (!dragHasContextRef(e) && !dragHasTaskRefs(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    if (!dragHovering) setDragHovering(true);
  }

  function handleDragLeave(e: ReactDragEvent<HTMLDivElement>) {
    // Fires for child-element transitions too; only clear when the
    // pointer truly leaves the host.
    if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
    setDragHovering(false);
  }

  function handleDrop(e: ReactDragEvent<HTMLDivElement>) {
    setDragHovering(false);
    const term = termRef.current;
    if (!term) return;

    // Multi-id tasks payload first — when present, iterate every id
    // and paste a space-separated chain of context mentions. This is the
    // path for "drag a marked Plan-pane row into the agent" (one or many).
    if (dragHasTaskRefs(e)) {
      const raw = e.dataTransfer.getData(WORK_ITEM_DRAG_MIME_VALUE);
      const refs = decodeTaskDragRefs(raw);
      if (refs.length > 0) {
        e.preventDefault();
        const text = refs.map(formatContextMention).join("");
        term.paste(text);
        term.focus();
        return;
      }
      // Fall through if the items slice was missing — older drag sources
      // may not embed it; still try the standalone CONTEXT_REF_MIME path.
    }

    const ref = readContextRef(e);
    if (!ref || (ref.kind === "file" && ref.path === "")) return;
    e.preventDefault();
    const text = formatContextMention(ref);
    term.paste(text);
    term.focus();
  }

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    // xterm's `fontFamily` is a one-shot constructor option, not a live
    // CSS-resolved value — we read `--font-mono` once at mount so the
    // terminal stays in lockstep with the rest of the app's mono surfaces.
    // Fall back to the literal stack if the token isn't present (tests, SSR).
    const resolvedMono =
      (typeof window !== "undefined"
        ? getComputedStyle(document.body).getPropertyValue("--font-mono").trim()
        : "") ||
      "ui-monospace, SFMono-Regular, Menlo, Consolas, \"Liberation Mono\", monospace";
    const term = new Terminal({
      fontFamily: resolvedMono,
      fontSize: 13,
      theme: XTERM_THEME,
      scrollback: 5000,
      cursorBlink: true,
      scrollSensitivity: 2,
      fastScrollModifier: "shift",
      fastScrollSensitivity: 4,
      scrollOnUserInput: true,
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);

    // File-path link provider — turns `path/to/file.ts:42` in terminal
    // output into a clickable, underlined link that opens the file in
    // a new editor tab via `onOpenFile`. Registered once per terminal;
    // the activate handler reads from refs so callback / worktree
    // updates are picked up live.
    const linkProviderDisp = installFilePathLinkProvider(term, {
      onActivate: (match: FilePathLinkActivation) => {
        const open = onOpenFileRef.current;
        if (!open) return;
        void resolveClickedPath(match.text, sessionIdRef.current, worktreePathRef.current).then(
          (abs) => {
            if (abs) open(abs, match.line, match.column);
          },
        );
      },
    });
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") {
        return true;
      }

      // Cmd+V (macOS paste shortcut) — xterm.js doesn't wire paste
      // itself, so read the clipboard and write through term.paste().
      // Use Electron's main-process clipboard (via IPC) — navigator.clipboard
      // rejects with "Document is not focused" on a fast Cmd-Tab → Cmd+V
      // and returns empty for non-text-primary flavors set by other apps.
      // Ctrl+V is NOT intercepted: it should reach the running CLI as a
      // literal ^V byte (0x16) so Claude Code's own paste handling
      // (including images) can run.
      if (event.metaKey && !event.ctrlKey && !event.altKey && event.key.toLowerCase() === "v") {
        event.preventDefault();
        void readClipboard().then((text) => {
          if (text) term.paste(text);
        }).catch((error) => {
          logUi("warn", "terminal paste: clipboard read failed", { error: String(error) });
        });
        return false;
      }

      // Shift+Enter — send ESC+CR (the Alt+Enter sequence) so Claude
      // Code's input treats it as a newline instead of a submit. xterm's
      // default would emit a bare \r for both Enter and Shift+Enter,
      // which Claude Code can't distinguish. Plain Enter falls through
      // unchanged so normal submits still work.
      if (event.key === "Enter" && event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        if (sessionIdRef.current) {
          void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({
            type: "input",
            bytes: btoa("\x1b\r"),
          }));
        }
        return false;
      }

      if (shouldHandleTerminalPageKey(event)) {
        const routeToTmuxHistory = transportMode === "tmux" && shouldRouteWheelToTmuxHistory({
          mode: modeRef.current,
          bufferType: term.buffer.active.type,
          mouseTrackingMode: term.modes.mouseTrackingMode,
        });

        if (routeToTmuxHistory) {
          if (sessionIdRef.current) {
            void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({
              type: "history-page",
              direction: event.key === "PageUp" ? "up" : "down",
            }));
          }
          setInteractionMode("history");
          return false;
        }

        if (term.buffer.active.type === "normal") {
          term.scrollPages(event.key === "PageUp" ? -1 : 1);
          return false;
        }

        return true;
      }

        if (transportMode === "tmux" && modeRef.current === "history" && shouldReturnTerminalToPrompt(event)) {
        if (sessionIdRef.current) {
          void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
        }
        setInteractionMode("live");
        term.focus();
        if (event.key === "Escape") {
          return false;
        }
      }

      // Plain Escape in live mode: Claude Code interprets a single
      // \x1b as "cancel the in-flight turn" but does NOT emit a Stop
      // hook for it, so oxplow's working-dot would stay Running until
      // the next user prompt. Notify the host so it can synthesize an
      // Interrupt hook. Don't intercept the byte itself — Claude still
      // needs to receive the \x1b through the normal onData path.
      if (
        event.key === "Escape" &&
        modeRef.current === "live" &&
        !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey
      ) {
        onUserInterrupt?.();
      }

      return true;
    });
    term.attachCustomWheelEventHandler((event) => {
      if (event.ctrlKey || event.metaKey) {
        return false;
      }

      const routeToTmuxHistory = transportMode === "tmux" && shouldRouteWheelToTmuxHistory({
        mode: modeRef.current,
        bufferType: term.buffer.active.type,
        mouseTrackingMode: term.modes.mouseTrackingMode,
      });

      if (!routeToTmuxHistory) {
        return true;
      }

      const lines = wheelDeltaToScrollLines(event);
      if (lines === 0) {
        return false;
      }

      if (sessionIdRef.current) {
        void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-scroll", lines }));
      }
      setInteractionMode("history");
      event.preventDefault();
      return false;
    });

    let disposed = false;
    let ro: ResizeObserver | null = null;
    const dataDisp = term.onData((data) => {
      if (sessionIdRef.current) {
        void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input", bytes: utf8ToBase64(data) }));
      }
    });
    const binaryDisp = term.onBinary((data) => {
      if (sessionIdRef.current) {
        void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input-binary", bytes: binaryToBase64(data) }));
      }
    });

    // Wait until the host has a real layout size, then open the terminal,
    // fit it, and only then open the WebSocket with the measured cols/rows
    // in the query string so the server can create the tmux window at the
    // correct size on first contact.
    const start = () => {
      if (disposed) return;
      if (host.clientWidth < 2 || host.clientHeight < 2) {
        requestAnimationFrame(start);
        return;
      }
      term.open(host);
      setTerm(term);
      try { fit.fit(); } catch {}
      if (term.cols < 2 || term.rows < 2) {
        requestAnimationFrame(start);
        return;
      }
      const handleMouseDown = () => {
        if (sessionIdRef.current) {
          void desktopBridge().sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
        }
        setInteractionMode("live");
        term.focus();
      };
      host.addEventListener("mousedown", handleMouseDown);

      // Catches the native Edit → Paste menu path (Electron's role:"paste"
      // fires a synthetic paste event on the focused element, which bubbles
      // up from xterm's hidden textarea to this host div) plus any paste
      // gesture we missed in the keydown handler above.
      const handlePaste = (event: ClipboardEvent) => {
        const text = event.clipboardData?.getData("text/plain");
        if (!text) return;
        event.preventDefault();
        event.stopPropagation();
        term.paste(text);
      };
      host.addEventListener("paste", handlePaste);

      // Right-click no longer pastes — the IA redesign moved every per-row
      // / per-pane action to a visible kebab `⋯` (see `.context/usability.md`).
      // The header-bar kebab below carries the Paste action; Cmd/Ctrl+V still
      // pastes via the keydown handler in the surrounding mousedown listener.

      // Direct-mode agents replay their scrollback synchronously from inside
      // the openTerminalSession handler, so terminal-event messages may reach
      // the renderer before the invoke response resolves and sessionIdRef is
      // set. Buffer them until the sessionId is known.
      const pendingEvents: TerminalEvent[] = [];
      const applyEvent = (event: TerminalEvent) => {
        try {
          const msg = JSON.parse(event.message);
          if (msg.type === "data" && typeof msg.bytes === "string") {
            const bin = atob(msg.bytes);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.write(bytes);
          }
        } catch {}
      };
      const unsubscribe = desktopBridge().onTerminalEvent((event) => {
        if (sessionIdRef.current === null) {
          pendingEvents.push(event);
          return;
        }
        if (event.sessionId !== sessionIdRef.current) return;
        applyEvent(event);
      });

      logUi("info", "opening terminal session", { paneTarget, cols: term.cols, rows: term.rows, transportMode });
      void desktopBridge().openTerminalSession(paneTarget, term.cols, term.rows, transportMode).then(({ sessionId, replayB64 }) => {
        if (disposed) {
          void desktopBridge().closeTerminalSession(sessionId);
          return;
        }
        sessionIdRef.current = sessionId;
        // Replay the session's ring buffer into the fresh xterm so
        // re-attaching to a long-running thread shows the same screen
        // state the user left it in (instead of a blank pane that
        // only fills as new output arrives).
        if (replayB64) {
          try {
            const bin = atob(replayB64);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.write(bytes);
          } catch {}
        }
        for (const event of pendingEvents) {
          if (event.sessionId === sessionId) applyEvent(event);
        }
        pendingEvents.length = 0;
        term.focus();
        if (transportMode === "tmux") {
          void desktopBridge().sendTerminalMessage(sessionId, JSON.stringify({ type: "history-exit" }));
        }
        setInteractionMode("live");
        logUi("info", "terminal session opened", { paneTarget, sessionId, transportMode });
      }).catch((error) => {
        logUi("error", "terminal session open failed", { paneTarget, error: String(error) });
      });

      // Debounce resizes so we don't spam tmux during a drag.
      let resizeTimer: number | null = null;
      ro = new ResizeObserver(() => {
        if (resizeTimer !== null) clearTimeout(resizeTimer);
        resizeTimer = window.setTimeout(() => {
          // Skip refits when the host is hidden (display:none) or otherwise
          // has no layout size — FitAddon would clamp to its minimum and we
          // would push a tiny resize at tmux, shrinking the underlying
          // window for real. See MainTabs/PaneHost: inactive tabs are
          // display:none'd rather than unmounted.
          if (host.clientWidth < 2 || host.clientHeight < 2) return;
          try {
            fit.fit();
            if (term.cols < 2 || term.rows < 2) return;
            if (sessionIdRef.current) {
              void desktopBridge().sendTerminalMessage(
                sessionIdRef.current,
                JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }),
              );
            }
          } catch {}
        }, 80);
      });
      ro.observe(host);

      const prevCleanup = cleanupRef.current;
      cleanupRef.current = () => {
        host.removeEventListener("mousedown", handleMouseDown);
        host.removeEventListener("paste", handlePaste);
        unsubscribe();
        prevCleanup?.();
      };
    };
    const cleanupRef: { current: (() => void) | null } = { current: null };
    start();

    return () => {
      disposed = true;
      cleanupRef.current?.();
      ro?.disconnect();
      dataDisp.dispose();
      binaryDisp.dispose();
      linkProviderDisp.dispose();
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      termRef.current = null;
      setTerm(null);
      if (sessionId) {
        void desktopBridge().closeTerminalSession(sessionId);
      }
      term.dispose();
    };
  }, [paneTarget, transportMode]);

  return (
    <div
      style={{ position: "relative", width: "100%", height: "100%", display: "flex", flexDirection: "column", minHeight: 0 }}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div ref={hostRef} style={{ flex: 1, minHeight: 0, width: "100%" }} />
      {comments && term ? (
        <TerminalCommentLayer
          term={term}
          streamId={comments.streamId}
          threadId={comments.threadId}
          targetKind={comments.targetKind}
          targetId={comments.targetId}
        />
      ) : null}
      {dragHovering ? (
        <div
          style={{
            position: "absolute",
            inset: 0,
            border: "2px dashed var(--accent)",
            background: "var(--accent-soft-bg)",
            pointerEvents: "none",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--text-primary)",
            fontSize: 13,
            zIndex: 5,
          }}
        >
          Drop to add to agent context
        </div>
      ) : null}
      {mode === "history" ? (
        <div
          style={{
            position: "absolute",
            right: 12,
            bottom: 12,
            padding: "6px 10px",
            border: "1px solid var(--border)",
            borderRadius: 6,
            background: "rgba(14, 14, 14, 0.92)",
            color: "var(--muted)",
            fontSize: 11,
            pointerEvents: "none",
          }}
        >
          History mode — click or type to return to the prompt
        </div>
      ) : null}
    </div>
  );
}


function binaryToBase64(data: string) {
  let binary = "";
  for (let i = 0; i < data.length; i++) {
    binary += String.fromCharCode(data.charCodeAt(i) & 0xff);
  }
  return btoa(binary);
}

/**
 * Encode a JS string as UTF-8 bytes, then base64. `btoa()` directly
 * rejects strings containing any character > U+00FF — pasting log
 * output with smart quotes / em-dashes / emoji used to throw
 * InvalidCharacterError and silently drop the paste. Going through
 * TextEncoder gets us proper UTF-8 round-tripping for the PTY.
 */
function utf8ToBase64(data: string) {
  const bytes = new TextEncoder().encode(data);
  let binary = "";
  // String.fromCharCode is fine for one byte at a time; chunked to
  // avoid the apply-with-large-array argument-limit pitfall.
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode.apply(null, Array.from(bytes.subarray(i, i + chunkSize)));
  }
  return btoa(binary);
}
