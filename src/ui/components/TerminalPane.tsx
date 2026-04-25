import { useEffect, useRef, useState, type DragEvent as ReactDragEvent } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import type { TerminalEvent } from "../../electron/ipc-contract.js";
import { logUi } from "../logger.js";
import {
  shouldHandleTerminalPageKey,
  shouldReturnTerminalToPrompt,
  shouldRouteWheelToTmuxHistory,
  wheelDeltaToScrollLines,
} from "../terminal-scroll.js";
import { subscribeAgentInput } from "../agent-input-bus.js";
import { dragHasContextRef, readContextRef } from "../agent-context-dnd.js";
import { formatContextMention } from "../agent-context-ref.js";

/**
 * Read the system clipboard as text. Prefers Electron's main-process
 * clipboard (via IPC) because navigator.clipboard.readText() in the
 * renderer rejects with "Document is not focused" on a fast Cmd-Tab →
 * Cmd+V and returns empty for non-text-primary flavors set by other
 * apps. Falls back to navigator.clipboard if the IPC path isn't wired.
 */
async function readClipboard(): Promise<string> {
  const api = window.oxplowApi as { clipboardReadText?: () => Promise<string> };
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
}: {
  paneTarget: string;
  visible: boolean;
  transportMode: "direct" | "tmux";
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const [mode, setMode] = useState<"live" | "history">("live");
  const modeRef = useRef<"live" | "history">("live");
  const [dragHovering, setDragHovering] = useState(false);

  function setInteractionMode(next: "live" | "history") {
    modeRef.current = next;
    setMode(next);
  }

  useEffect(() => {
    if (!visible) return;
    termRef.current?.focus();
    if (transportMode === "tmux" && sessionIdRef.current) {
      void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
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
    if (!dragHasContextRef(e)) return;
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
    const ref = readContextRef(e);
    if (!ref || (ref.kind === "file" && ref.path === "")) return;
    e.preventDefault();
    const text = formatContextMention(ref);
    const term = termRef.current;
    if (!term) return;
    term.paste(text);
    term.focus();
  }

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 13,
      theme: { background: "#0e0e0e", foreground: "#e6e6e6" },
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
          void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({
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
            void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({
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
          void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
        }
        setInteractionMode("live");
        term.focus();
        if (event.key === "Escape") {
          return false;
        }
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
        void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-scroll", lines }));
      }
      setInteractionMode("history");
      event.preventDefault();
      return false;
    });

    let disposed = false;
    let ro: ResizeObserver | null = null;
    const dataDisp = term.onData((data) => {
      if (sessionIdRef.current) {
        void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input", bytes: btoa(data) }));
      }
    });
    const binaryDisp = term.onBinary((data) => {
      if (sessionIdRef.current) {
        void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input-binary", bytes: binaryToBase64(data) }));
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
      try { fit.fit(); } catch {}
      if (term.cols < 2 || term.rows < 2) {
        requestAnimationFrame(start);
        return;
      }
      const handleMouseDown = () => {
        if (sessionIdRef.current) {
          void window.oxplowApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
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

      // Electron disables the browser's default context menu in renderers,
      // so a plain right-click shows nothing. Match the tmux / iTerm2
      // convention — right-click = paste — instead of wiring up a full menu
      // for a single item.
      const handleContextMenu = (event: MouseEvent) => {
        event.preventDefault();
        void readClipboard().then((text) => {
          if (text) term.paste(text);
        }).catch((error) => {
          logUi("warn", "terminal paste: clipboard read failed", { error: String(error) });
        });
      };
      host.addEventListener("contextmenu", handleContextMenu);

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
      const unsubscribe = window.oxplowApi.onTerminalEvent((event) => {
        if (sessionIdRef.current === null) {
          pendingEvents.push(event);
          return;
        }
        if (event.sessionId !== sessionIdRef.current) return;
        applyEvent(event);
      });

      logUi("info", "opening terminal session", { paneTarget, cols: term.cols, rows: term.rows, transportMode });
      void window.oxplowApi.openTerminalSession(paneTarget, term.cols, term.rows, transportMode).then((sessionId) => {
        if (disposed) {
          void window.oxplowApi.closeTerminalSession(sessionId);
          return;
        }
        sessionIdRef.current = sessionId;
        for (const event of pendingEvents) {
          if (event.sessionId === sessionId) applyEvent(event);
        }
        pendingEvents.length = 0;
        term.focus();
        if (transportMode === "tmux") {
          void window.oxplowApi.sendTerminalMessage(sessionId, JSON.stringify({ type: "history-exit" }));
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
              void window.oxplowApi.sendTerminalMessage(
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
        host.removeEventListener("contextmenu", handleContextMenu);
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
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      termRef.current = null;
      if (sessionId) {
        void window.oxplowApi.closeTerminalSession(sessionId);
      }
      term.dispose();
    };
  }, [paneTarget, transportMode]);

  return (
    <div
      style={{ position: "relative", width: "100%", height: "100%" }}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <div ref={hostRef} style={{ width: "100%", height: "100%" }} />
      {dragHovering ? (
        <div
          style={{
            position: "absolute",
            inset: 0,
            border: "2px dashed var(--color-status-info, #5a8ac9)",
            background: "rgba(90, 138, 201, 0.10)",
            pointerEvents: "none",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            color: "var(--color-text, #ddd)",
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
