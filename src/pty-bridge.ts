import type { WebSocket } from "ws";
import { spawn } from "node-pty";
import { resizeWindow } from "./tmux.js";

interface ClientMsg {
  type: "input" | "resize";
  bytes?: string;
  cols?: number;
  rows?: number;
}

export function attachPane(ws: WebSocket, paneTarget: string, cols: number, rows: number) {
  const pty = spawn("tmux", ["attach-session", "-t", paneTarget], {
    name: "xterm-256color",
    cols,
    rows,
    env: process.env as Record<string, string>,
  });

  pty.onData((data) => {
    if (ws.readyState !== ws.OPEN) return;
    ws.send(JSON.stringify({ type: "data", bytes: Buffer.from(data).toString("base64") }));
  });

  pty.onExit(() => {
    try {
      ws.close();
    } catch {}
  });

  ws.on("message", (raw) => {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(raw.toString());
    } catch {
      return;
    }
    if (msg.type === "input" && msg.bytes) {
      pty.write(Buffer.from(msg.bytes, "base64").toString("utf8"));
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
    try { pty.kill(); } catch {}
  });
}
