import { spawn } from "node-pty";
import type { Logger } from "../core/logger.js";
import type { BridgeSocket } from "./bridge-socket.js";
import { capturePaneHistory, copyModePage, copyModeScroll, exitCopyMode, refreshClients, resizeWindow } from "./tmux.js";

interface ClientMsg {
  type: "input" | "input-binary" | "resize" | "history-page" | "history-scroll" | "history-exit";
  bytes?: string;
  cols?: number;
  rows?: number;
  direction?: "up" | "down";
  lines?: number;
}

export function attachPane(ws: BridgeSocket, paneTarget: string, cols: number, rows: number, logger?: Logger) {
  logger?.info("attaching pane bridge", { paneTarget, cols, rows });
  const initialHistory = capturePaneHistory(paneTarget);
  if (initialHistory && ws.readyState === ws.OPEN) {
    ws.send(JSON.stringify({ type: "data", bytes: Buffer.from(initialHistory).toString("base64") }));
    logger?.debug("sent initial pane history", { paneTarget, bytes: Buffer.byteLength(initialHistory) });
  }
  const pty = spawn("tmux", ["attach-session", "-t", paneTarget], {
    name: "xterm-256color",
    cols,
    rows,
    env: process.env as Record<string, string>,
  });

  // Force tmux to repaint the freshly-attached client. Without this, when the
  // inner program (e.g. claude that just spawned) hasn't produced output yet,
  // tmux's attach is silent and xterm shows whatever pixels the host DOM was
  // last painted with — which on a transport-mode toggle looks like the click
  // did nothing until the user types and provokes output.
  refreshClients();

  pty.onData((data) => {
    if (ws.readyState !== ws.OPEN) return;
    ws.send(JSON.stringify({ type: "data", bytes: Buffer.from(data).toString("base64") }));
  });

  pty.onExit(() => {
    logger?.info("pty exited", { paneTarget });
    try {
      ws.close();
    } catch {}
  });

  ws.on("message", (raw) => {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      logger?.warn("failed to parse websocket message", { paneTarget });
      return;
    }
    if (msg.type === "input" && msg.bytes) {
      pty.write(Buffer.from(msg.bytes, "base64").toString("utf8"));
    } else if (msg.type === "input-binary" && msg.bytes) {
      pty.write(Buffer.from(msg.bytes, "base64").toString("binary"));
    } else if (msg.type === "history-exit") {
      exitCopyMode(paneTarget);
    } else if (msg.type === "history-page" && msg.direction) {
      copyModePage(paneTarget, msg.direction);
    } else if (msg.type === "history-scroll" && typeof msg.lines === "number") {
      copyModeScroll(paneTarget, msg.lines);
    } else if (msg.type === "resize" && msg.cols && msg.rows) {
      // Reject absurdly-small resizes: a hidden xterm (display:none) can
      // fit-down to a couple of cells, and propagating that to tmux shrinks
      // the real window. With `window-size manual` tmux won't grow it back.
      if (msg.cols < 20 || msg.rows < 5) return;
      try { pty.resize(msg.cols, msg.rows); } catch {}
      // window-size is manual, so we have to drive tmux's resize explicitly.
      resizeWindow(paneTarget, msg.cols, msg.rows);
    }
  });

  ws.on("close", () => {
    logger?.info("pane websocket closed", { paneTarget });
    try { pty.kill(); } catch {}
  });
}
