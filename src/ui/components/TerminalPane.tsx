import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";

export function TerminalPane({ paneTarget }: { paneTarget: string }) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      fontSize: 13,
      theme: { background: "#0e0e0e", foreground: "#e6e6e6" },
      scrollback: 5000,
      cursorBlink: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    let disposed = false;
    let ws: WebSocket | null = null;
    let ro: ResizeObserver | null = null;
    const dataDisp = term.onData((data) => {
      if (ws && ws.readyState === ws.OPEN) {
        ws.send(JSON.stringify({ type: "input", bytes: btoa(data) }));
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

      const proto = location.protocol === "https:" ? "wss:" : "ws:";
      const url = `${proto}//${location.host}/ws?pane=${encodeURIComponent(paneTarget)}&cols=${term.cols}&rows=${term.rows}`;
      ws = new WebSocket(url);

      ws.onmessage = (ev) => {
        try {
          const msg = JSON.parse(ev.data);
          if (msg.type === "data" && typeof msg.bytes === "string") {
            const bin = atob(msg.bytes);
            const bytes = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            term.write(bytes);
          }
        } catch {}
      };

      // Debounce resizes so we don't spam tmux during a drag.
      let resizeTimer: number | null = null;
      ro = new ResizeObserver(() => {
        if (resizeTimer !== null) clearTimeout(resizeTimer);
        resizeTimer = window.setTimeout(() => {
          try {
            fit.fit();
            if (ws && ws.readyState === ws.OPEN) {
              ws.send(JSON.stringify({ type: "resize", cols: term.cols, rows: term.rows }));
            }
          } catch {}
        }, 80);
      });
      ro.observe(host);
    };
    start();

    return () => {
      disposed = true;
      ro?.disconnect();
      dataDisp.dispose();
      try { ws?.close(); } catch {}
      term.dispose();
    };
  }, [paneTarget]);

  return <div ref={hostRef} style={{ width: "100%", height: "100%" }} />;
}
