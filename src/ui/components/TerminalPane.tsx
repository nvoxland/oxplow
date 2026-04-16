import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { logUi } from "../logger.js";
import {
  shouldHandleTerminalPageKey,
  shouldReturnTerminalToPrompt,
  shouldRouteWheelToTmuxHistory,
  wheelDeltaToScrollLines,
} from "../terminal-scroll.js";

export function TerminalPane({ paneTarget, visible }: { paneTarget: string; visible: boolean }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const termRef = useRef<Terminal | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const [mode, setMode] = useState<"live" | "history">("live");
  const [transportMode, setTransportMode] = useState<"direct" | "tmux">("direct");
  const modeRef = useRef<"live" | "history">("live");

  function setInteractionMode(next: "live" | "history") {
    modeRef.current = next;
    setMode(next);
  }

  useEffect(() => {
    setTransportMode("direct");
  }, [paneTarget]);

  useEffect(() => {
    if (!visible) return;
    termRef.current?.focus();
    if (transportMode === "tmux" && sessionIdRef.current) {
      void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
    }
    setInteractionMode("live");
  }, [paneTarget, transportMode, visible]);

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

      if (shouldHandleTerminalPageKey(event)) {
        const routeToTmuxHistory = transportMode === "tmux" && shouldRouteWheelToTmuxHistory({
          mode: modeRef.current,
          bufferType: term.buffer.active.type,
          mouseTrackingMode: term.modes.mouseTrackingMode,
        });

        if (routeToTmuxHistory) {
          if (sessionIdRef.current) {
            void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({
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
          void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
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
        void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-scroll", lines }));
      }
      setInteractionMode("history");
      event.preventDefault();
      return false;
    });

    let disposed = false;
    let ro: ResizeObserver | null = null;
    const dataDisp = term.onData((data) => {
      if (sessionIdRef.current) {
        void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input", bytes: btoa(data) }));
      }
    });
    const binaryDisp = term.onBinary((data) => {
      if (sessionIdRef.current) {
        void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "input-binary", bytes: binaryToBase64(data) }));
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
          void window.newdeApi.sendTerminalMessage(sessionIdRef.current, JSON.stringify({ type: "history-exit" }));
        }
        setInteractionMode("live");
        term.focus();
      };
      host.addEventListener("mousedown", handleMouseDown);

      const unsubscribe = window.newdeApi.onTerminalEvent((event) => {
        if (event.sessionId !== sessionIdRef.current) return;
        try {
          const msg = JSON.parse(event.message);
          if (msg.type === "data" && typeof msg.bytes === "string") {
            const bin = atob(msg.bytes);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.write(bytes);
          }
        } catch {}
      });

      logUi("info", "opening terminal session", { paneTarget, cols: term.cols, rows: term.rows, transportMode });
      void window.newdeApi.openTerminalSession(paneTarget, term.cols, term.rows, transportMode).then((sessionId) => {
        if (disposed) {
          void window.newdeApi.closeTerminalSession(sessionId);
          return;
        }
        sessionIdRef.current = sessionId;
        term.focus();
        if (transportMode === "tmux") {
          void window.newdeApi.sendTerminalMessage(sessionId, JSON.stringify({ type: "history-exit" }));
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
              void window.newdeApi.sendTerminalMessage(
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
        void window.newdeApi.closeTerminalSession(sessionId);
      }
      term.dispose();
    };
  }, [paneTarget, transportMode]);

  return (
    <div style={{ position: "relative", width: "100%", height: "100%" }}>
      <div ref={hostRef} style={{ width: "100%", height: "100%" }} />
      <div style={{ position: "absolute", top: 10, right: 10, zIndex: 2, display: "flex", gap: 8 }}>
        {transportMode === "direct" ? (
          <button
            onClick={() => setTransportMode("tmux")}
            style={modeButtonStyle}
          >
            Open in tmux
          </button>
        ) : (
          <button
            onClick={() => setTransportMode("direct")}
            style={modeButtonStyle}
          >
            Use direct mode
          </button>
        )}
      </div>
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

const modeButtonStyle = {
  padding: "6px 10px",
  border: "1px solid var(--border)",
  borderRadius: 6,
  background: "rgba(14, 14, 14, 0.92)",
  color: "inherit",
  cursor: "pointer",
  font: "inherit",
  fontSize: 11,
} satisfies React.CSSProperties;

function binaryToBase64(data: string) {
  let binary = "";
  for (let i = 0; i < data.length; i++) {
    binary += String.fromCharCode(data.charCodeAt(i) & 0xff);
  }
  return btoa(binary);
}
