import { spawn as spawnPty } from "node-pty";
import type { Logger } from "../core/logger.js";

/** Minimal subset of node-pty's IPty that AgentPty uses. Exposed as an
 *  interface so tests can inject a fake. */
export interface IPtyLike {
  onData(cb: (data: string) => void): void;
  onExit(cb: () => void): void;
  write(data: string): void;
  resize(cols: number, rows: number): void;
  kill(): void;
}

export interface AgentPtySpec {
  command: string;
  cwd: string;
  cols: number;
  rows: number;
}

export type PtyFactory = (spec: AgentPtySpec) => IPtyLike;

import type { BridgeSocket } from "./bridge-socket.js";

// Backwards-compatible alias (was named AttachedSocket; the surface is now
// shared across pty/lsp bridges as BridgeSocket).
type AttachedSocket = BridgeSocket;

interface ClientMsg {
  type: "input" | "input-binary" | "resize";
  bytes?: string;
  cols?: number;
  rows?: number;
}

interface StoreOptions {
  /** Max bytes to retain in the replay buffer. Defaults to ~4 MB. */
  ringBufferBytes?: number;
}

const DEFAULT_RING_BYTES = 4_000_000;

function defaultSpawnPty(spec: AgentPtySpec): IPtyLike {
  return spawnPty("sh", ["-lc", spec.command], {
    name: "xterm-256color",
    cwd: spec.cwd,
    cols: spec.cols,
    rows: spec.rows,
    env: { ...process.env, COLORTERM: "truecolor" } as Record<string, string>,
  }) as IPtyLike;
}

export class AgentPty {
  private readonly ring: string[] = [];
  private ringBytes = 0;
  private attached: AttachedSocket | null = null;
  private attachedListeners: {
    message: (raw: unknown) => void;
    close: () => void;
  } | null = null;
  private closed = false;
  private readonly maxBytes: number;

  constructor(
    readonly threadId: string,
    private readonly pty: IPtyLike,
    opts: { ringBufferBytes?: number; logger?: Logger; onExit?: () => void } = {},
  ) {
    this.maxBytes = opts.ringBufferBytes ?? DEFAULT_RING_BYTES;
    pty.onData((chunk) => this.handleOutput(chunk));
    pty.onExit(() => {
      this.closed = true;
      this.attached?.close();
      this.attached = null;
      opts.onExit?.();
    });
  }

  private handleOutput(chunk: string): void {
    this.ring.push(chunk);
    this.ringBytes += Buffer.byteLength(chunk, "utf8");
    while (this.ringBytes > this.maxBytes && this.ring.length > 1) {
      const head = this.ring.shift()!;
      this.ringBytes -= Buffer.byteLength(head, "utf8");
    }
    // If single chunk exceeds limit, trim from the left.
    if (this.ringBytes > this.maxBytes && this.ring.length === 1) {
      const only = this.ring[0];
      const trimmed = only.slice(-this.maxBytes);
      this.ring[0] = trimmed;
      this.ringBytes = Buffer.byteLength(trimmed, "utf8");
    }
    if (this.attached) {
      this.sendData(this.attached, chunk);
    }
  }

  attach(socket: AttachedSocket, cols: number, rows: number): void {
    if (this.closed) {
      socket.close();
      return;
    }
    if (this.attached) {
      this.detach();
    }
    this.attached = socket;
    this.pty.resize(cols, rows);
    if (this.ring.length > 0) {
      const replay = this.ring.join("");
      this.sendData(socket, replay);
    }
    const onMessage = (raw: unknown) => this.handleSocketMessage(String(raw));
    const onClose = () => {
      if (this.attached === socket) this.detach();
    };
    socket.on("message", onMessage);
    socket.on("close", onClose);
    this.attachedListeners = { message: onMessage, close: onClose };
  }

  private detach(): void {
    const current = this.attached;
    const listeners = this.attachedListeners;
    this.attached = null;
    this.attachedListeners = null;
    if (current && listeners) {
      const removeListener =
        (current.removeListener ?? current.off)?.bind(current);
      removeListener?.("message", listeners.message);
      removeListener?.("close", listeners.close);
    }
    if (current && current.readyState === current.OPEN) {
      current.close();
    }
  }

  private handleSocketMessage(raw: string): void {
    let msg: ClientMsg;
    try {
      msg = JSON.parse(raw);
    } catch {
      return;
    }
    if (msg.type === "input" && msg.bytes) {
      this.pty.write(Buffer.from(msg.bytes, "base64").toString("utf8"));
    } else if (msg.type === "input-binary" && msg.bytes) {
      this.pty.write(Buffer.from(msg.bytes, "base64").toString("binary"));
    } else if (msg.type === "resize" && msg.cols && msg.rows) {
      if (msg.cols < 20 || msg.rows < 5) return;
      this.pty.resize(msg.cols, msg.rows);
    }
  }

  private sendData(socket: AttachedSocket, data: string): void {
    socket.send(
      JSON.stringify({ type: "data", bytes: Buffer.from(data, "utf8").toString("base64") }),
    );
  }

  dispose(): void {
    this.detach();
    if (!this.closed) {
      this.closed = true;
      try { this.pty.kill(); } catch {}
    }
  }

  get isClosed(): boolean {
    return this.closed;
  }
}

export class AgentPtyStore {
  private readonly ptys = new Map<string, AgentPty>();
  private readonly options: StoreOptions;

  constructor(
    private readonly spawn: PtyFactory = defaultSpawnPty,
    options: StoreOptions = {},
  ) {
    this.options = options;
  }

  ensure(threadId: string, spec: AgentPtySpec, logger?: Logger): AgentPty {
    const existing = this.ptys.get(threadId);
    if (existing && !existing.isClosed) return existing;
    const pty = this.spawn(spec);
    const agent = new AgentPty(threadId, pty, {
      ringBufferBytes: this.options.ringBufferBytes,
      logger,
      onExit: () => {
        this.ptys.delete(threadId);
      },
    });
    this.ptys.set(threadId, agent);
    return agent;
  }

  get(threadId: string): AgentPty | null {
    const agent = this.ptys.get(threadId);
    if (!agent || agent.isClosed) return null;
    return agent;
  }

  dispose(threadId: string): void {
    const agent = this.ptys.get(threadId);
    if (!agent) return;
    this.ptys.delete(threadId);
    agent.dispose();
  }

  disposeAll(): void {
    for (const agent of this.ptys.values()) agent.dispose();
    this.ptys.clear();
  }
}
