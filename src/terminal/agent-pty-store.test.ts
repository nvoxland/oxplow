import { EventEmitter } from "node:events";
import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { AgentPtyStore, type IPtyLike } from "./agent-pty-store.js";

class FakePty extends EventEmitter implements IPtyLike {
  alive = true;
  writes: string[] = [];
  resized: { cols: number; rows: number } | null = null;
  killed = false;

  onData(cb: (data: string) => void): void {
    this.on("data", cb);
  }
  onExit(cb: () => void): void {
    this.on("exit", cb);
  }
  write(data: string): void {
    this.writes.push(data);
  }
  resize(cols: number, rows: number): void {
    this.resized = { cols, rows };
  }
  kill(): void {
    this.killed = true;
    this.alive = false;
    this.emit("exit");
  }

  emitData(chunk: string): void {
    this.emit("data", chunk);
  }
}

class FakeSocket extends EventEmitter {
  readonly OPEN = 1;
  readonly CLOSED = 3;
  readyState = this.OPEN;
  sent: string[] = [];

  send(message: string): void {
    if (this.readyState !== this.OPEN) return;
    this.sent.push(message);
  }

  close(): void {
    if (this.readyState !== this.OPEN) return;
    this.readyState = this.CLOSED;
    this.emit("close");
  }
}

function decodeDataMessages(sent: string[]): string[] {
  return sent
    .map((raw) => {
      try {
        const msg = JSON.parse(raw);
        if (msg.type === "data" && typeof msg.bytes === "string") {
          return Buffer.from(msg.bytes, "base64").toString("utf8");
        }
        return "";
      } catch {
        return "";
      }
    })
    .filter((s) => s.length > 0);
}

let store: AgentPtyStore;
let lastPty: FakePty | null;
let spawnCount: number;

beforeEach(() => {
  lastPty = null;
  spawnCount = 0;
  store = new AgentPtyStore(() => {
    spawnCount += 1;
    const pty = new FakePty();
    lastPty = pty;
    return pty;
  });
});

afterEach(() => {
  store.disposeAll();
});

describe("AgentPtyStore", () => {
  test("ensure spawns once per thread and returns the same pty on re-ensure", () => {
    const a = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const b = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    expect(spawnCount).toBe(1);
    expect(a).toBe(b);
  });

  test("attach replays buffered output to a new socket", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    lastPty!.emitData("hello\n");
    lastPty!.emitData("world\n");
    const socket = new FakeSocket();
    pty.attach(socket as any, 100, 30);
    const replayed = decodeDataMessages(socket.sent).join("");
    expect(replayed).toContain("hello");
    expect(replayed).toContain("world");
  });

  test("live output reaches the currently attached socket", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const socket = new FakeSocket();
    pty.attach(socket as any, 80, 24);
    socket.sent.length = 0;
    lastPty!.emitData("after-attach");
    expect(decodeDataMessages(socket.sent).join("")).toContain("after-attach");
  });

  test("detaching a socket does not kill the pty; re-attach sees prior output", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const first = new FakeSocket();
    pty.attach(first as any, 80, 24);
    lastPty!.emitData("keep-me\n");
    first.close();
    expect(lastPty!.killed).toBe(false);
    const second = new FakeSocket();
    pty.attach(second as any, 80, 24);
    expect(decodeDataMessages(second.sent).join("")).toContain("keep-me");
  });

  test("attaching a second socket closes the previously attached socket", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const first = new FakeSocket();
    pty.attach(first as any, 80, 24);
    const second = new FakeSocket();
    pty.attach(second as any, 80, 24);
    expect(first.readyState).toBe(first.CLOSED);
    expect(second.readyState).toBe(second.OPEN);
  });

  test("socket input-messages are forwarded to the pty; resize drives pty.resize", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const socket = new FakeSocket();
    pty.attach(socket as any, 80, 24);
    socket.emit("message", JSON.stringify({ type: "input", bytes: Buffer.from("hi").toString("base64") }));
    expect(lastPty!.writes.join("")).toContain("hi");
    socket.emit("message", JSON.stringify({ type: "resize", cols: 120, rows: 40 }));
    expect(lastPty!.resized).toEqual({ cols: 120, rows: 40 });
  });

  test("dispose kills the pty and drops it from the store", () => {
    store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const firstPty = lastPty!;
    store.dispose("b-1");
    expect(firstPty.killed).toBe(true);
    store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    expect(spawnCount).toBe(2);
  });

  test("pty exit removes it from the store and closes the attached socket", () => {
    const pty = store.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    const socket = new FakeSocket();
    pty.attach(socket as any, 80, 24);
    lastPty!.emit("exit");
    expect(socket.readyState).toBe(socket.CLOSED);
    expect(store.get("b-1")).toBeNull();
  });

  test("ring buffer caps bytes so long-running agents don't grow unbounded", () => {
    const smallStore = new AgentPtyStore(
      () => {
        const pty = new FakePty();
        lastPty = pty;
        return pty;
      },
      { ringBufferBytes: 100 },
    );
    const pty = smallStore.ensure("b-1", { command: "x", cwd: "/", cols: 80, rows: 24 });
    lastPty!.emitData("x".repeat(200));
    const socket = new FakeSocket();
    pty.attach(socket as any, 80, 24);
    const replayed = decodeDataMessages(socket.sent).join("");
    expect(replayed.length).toBeLessThanOrEqual(100);
    smallStore.disposeAll();
  });
});
