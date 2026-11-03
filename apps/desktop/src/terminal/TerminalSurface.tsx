/// The terminal's surface (`.context/terminal.md` "The terminal viz and
/// its host"): the one xterm.js host — the screen, the font it opens in,
/// the keys and gestures oxplow adds (Cmd+V, Shift+Enter, Page Up/Down,
/// Escape, paste, the agent input bus, file-path links) —
/// and nothing about a session. What the person types comes out through
/// `onData` / `onBinary`, a fit through `onResize`; `onOpened` hands the
/// opened terminal to whoever writes to it (`useTerminalSession`).
import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

import { desktopBridge } from "../api.js";
import { logUi } from "../logger.js";
import { shouldHandleTerminalPageKey } from "../terminal-scroll.js";
import { subscribeAgentInput } from "../agent-input-bus.js";
import { installFilePathLinkProvider, type FilePathLinkActivation } from "../terminal-link-provider.js";

/** The terminal's font size, px: its cells are measured in it. */
const TERMINAL_FONT_SIZE = 13;

export const XTERM_THEME = {
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
/// worktree root when the cwd can't be determined (dead session, unsupported
/// platform).
async function resolveClickedPath(
  text: string,
  cwd: (() => Promise<string | null>) | undefined,
  worktree: string | undefined,
): Promise<string | null> {
  if (!text) return null;
  if (text.startsWith("/")) return text;
  if (text.startsWith("~/")) return null;
  const live = cwd ? await cwd() : null;
  return resolveAgainstWorktree(text, live ?? worktree);
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

export interface TerminalSurfaceProps {
  /** On screen: it takes the keys and the agent input bus. */
  shown: boolean;
  /** The terminal opened at a real size: write to it, and open the
   *  session at its `cols` × `rows`. */
  onOpened(term: Terminal): void;
  /** What the person typed (`input`), or binary input. */
  onData(data: string): void;
  onBinary(data: string): void;
  /** It fit to a new size. */
  onResize(cols: number, rows: number): void;
  /** Plain Escape: Claude Code cancels its turn on it without a Stop
   *  hook, so the host records the interrupt. The byte still goes. */
  onInterrupt?(): void;
  /** Relative paths in the output resolve against this, unless `cwd`
   *  answers the session's live directory. */
  worktreePath?: string;
  cwd?(): Promise<string | null>;
  /** A clicked file path, absolute, with its line and column. */
  onOpenFile?(absPath: string, line?: number, column?: number): void;
  /** Which path-shaped tokens are links (real workspace files); all when
   *  omitted. */
  isLinkablePath?(path: string): boolean;
}

export function TerminalSurface({
  shown,
  onOpened,
  onData,
  onBinary,
  onResize,
  onInterrupt,
  worktreePath,
  cwd,
  onOpenFile,
  isLinkablePath,
}: TerminalSurfaceProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  // Read at event time: the terminal is set up once.
  const props = useRef({ onOpened, onData, onBinary, onResize, onInterrupt, worktreePath, cwd, onOpenFile, isLinkablePath });
  props.current = { onOpened, onData, onBinary, onResize, onInterrupt, worktreePath, cwd, onOpenFile, isLinkablePath };

  useEffect(() => {
    if (shown) termRef.current?.focus();
  }, [shown]);

  // The agent input bus (`agent_input.write`, Repair with the Agent) fills
  // the terminal on screen: `term.paste` writes through xterm's input, so
  // `onData` sends it.
  useEffect(() => {
    if (!shown) return;
    return subscribeAgentInput((text) => {
      const term = termRef.current;
      if (!term) return;
      term.paste(text);
      term.focus();
    });
  }, [shown]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    // xterm's `fontFamily` is read once: `--font-mono`, else a mono stack.
    const resolvedMono =
      (typeof window !== "undefined" ? getComputedStyle(document.body).getPropertyValue("--font-mono").trim() : "") ||
      'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace';
    const term = new Terminal({
      fontFamily: resolvedMono,
      fontSize: TERMINAL_FONT_SIZE,
      theme: XTERM_THEME,
      scrollback: 5000,
      cursorBlink: true,
      scrollSensitivity: 2,
      fastScrollSensitivity: 4,
      scrollOnUserInput: true,
    });
    termRef.current = term;
    const fit = new FitAddon();
    term.loadAddon(fit);

    const linkProviderDisp = installFilePathLinkProvider(term, {
      onActivate: (match: FilePathLinkActivation) => {
        const open = props.current.onOpenFile;
        if (!open) return;
        void resolveClickedPath(match.text, props.current.cwd, props.current.worktreePath).then((abs) => {
          if (abs) open(abs, match.line, match.column);
        });
      },
      validatePath: (path: string) => props.current.isLinkablePath?.(path) ?? true,
    });
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return true;
      // Cmd+V: xterm doesn't paste itself. Ctrl+V stays a ^V byte for the
      // program (Claude Code pastes images on it).
      if (event.metaKey && !event.ctrlKey && !event.altKey && event.key.toLowerCase() === "v") {
        event.preventDefault();
        void readClipboard()
          .then((text) => {
            if (text) term.paste(text);
          })
          .catch((error) => logUi("warn", "terminal paste: clipboard read failed", { error: String(error) }));
        return false;
      }
      // Shift+Enter: ESC+CR, a newline to Claude Code's input, not a submit.
      if (event.key === "Enter" && event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey) {
        event.preventDefault();
        props.current.onData("\x1b\r");
        return false;
      }
      if (shouldHandleTerminalPageKey(event)) {
        if (term.buffer.active.type === "normal") {
          term.scrollPages(event.key === "PageUp" ? -1 : 1);
          return false;
        }
        return true;
      }
      if (event.key === "Escape" && !event.metaKey && !event.ctrlKey && !event.altKey && !event.shiftKey) {
        props.current.onInterrupt?.();
      }
      return true;
    });
    // A modified wheel is the page's (zoom), not the terminal's scroll.
    term.attachCustomWheelEventHandler((event) => !(event.ctrlKey || event.metaKey));
    const dataDisp = term.onData((data) => props.current.onData(data));
    const binaryDisp = term.onBinary((data) => props.current.onBinary(data));

    let disposed = false;
    let ro: ResizeObserver | null = null;
    const cleanups: (() => void)[] = [];
    // Open once the host has a size, fit, then hand it over: the session
    // starts at the measured size.
    const start = () => {
      if (disposed) return;
      if (host.clientWidth < 2 || host.clientHeight < 2) {
        requestAnimationFrame(start);
        return;
      }
      term.open(host);
      try {
        fit.fit();
      } catch {}
      if (term.cols < 2 || term.rows < 2) {
        requestAnimationFrame(start);
        return;
      }
      const handleMouseDown = () => term.focus();
      host.addEventListener("mousedown", handleMouseDown);
      // The native Edit → Paste and any paste the key handler missed.
      const handlePaste = (event: ClipboardEvent) => {
        const text = event.clipboardData?.getData("text/plain");
        if (!text) return;
        event.preventDefault();
        event.stopPropagation();
        term.paste(text);
      };
      host.addEventListener("paste", handlePaste);
      cleanups.push(() => {
        host.removeEventListener("mousedown", handleMouseDown);
        host.removeEventListener("paste", handlePaste);
      });
      props.current.onOpened(term);
      // Debounced refits; none while hidden (`display: none` has no size,
      // and a fit would shrink the program's screen for real).
      let resizeTimer: number | null = null;
      ro = new ResizeObserver(() => {
        if (resizeTimer !== null) clearTimeout(resizeTimer);
        resizeTimer = window.setTimeout(() => {
          if (host.clientWidth < 2 || host.clientHeight < 2) return;
          try {
            fit.fit();
            if (term.cols < 2 || term.rows < 2) return;
            props.current.onResize(term.cols, term.rows);
          } catch {}
        }, 80);
      });
      ro.observe(host);
    };
    // xterm measures its cells when it opens, and again only after a
    // resize: opened in the fallback font, its rows overflow once the mono
    // web font lands (tsk1042). So it opens once the font is in.
    const fontLoaded: Promise<unknown> =
      typeof document !== "undefined" && document.fonts
        ? document.fonts.load(`${TERMINAL_FONT_SIZE}px ${resolvedMono}`).catch(() => undefined)
        : Promise.resolve();
    void fontLoaded.then(start);

    return () => {
      disposed = true;
      cleanups.forEach((c) => c());
      ro?.disconnect();
      dataDisp.dispose();
      binaryDisp.dispose();
      linkProviderDisp.dispose();
      termRef.current = null;
      term.dispose();
    };
  }, []);

  return (
    <div
      style={{ position: "relative", width: "100%", flex: 1, display: "flex", flexDirection: "column", minHeight: 0 }}
    >
      {/* The gutter is on this wrapper, never the mount: FitAddon measures
          the mount, and padding it makes the row count oscillate. */}
      <div
        style={{
          flex: 1,
          minHeight: 0,
          minWidth: 0,
          width: "100%",
          boxSizing: "border-box",
          padding: 8,
          background: XTERM_THEME.background,
          display: "flex",
        }}
      >
        <div ref={hostRef} data-testid="terminal-mount" style={{ flex: 1, minHeight: 0, minWidth: 0 }} />
      </div>
    </div>
  );
}
