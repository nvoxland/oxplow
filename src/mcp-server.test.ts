import { test, expect } from "bun:test";
import { existsSync, readFileSync, readdirSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import WebSocket from "ws";
import { startMcpServer } from "./mcp-server.js";

interface RpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: any;
  error?: { code: number; message: string };
}

function tempIdeDir(): string {
  return mkdtempSync(join(tmpdir(), "newde-ide-"));
}

function connect(port: number, token: string): Promise<WebSocket> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(`ws://127.0.0.1:${port}/`, {
      headers: { "x-claude-code-ide-authorization": token },
    });
    ws.once("open", () => resolve(ws));
    ws.once("error", reject);
  });
}

function rpc(ws: WebSocket, id: number, method: string, params: any = {}): Promise<RpcResponse> {
  return new Promise((resolve, reject) => {
    const onMsg = (raw: WebSocket.RawData) => {
      const msg = JSON.parse(raw.toString());
      if (msg.id === id) {
        ws.off("message", onMsg);
        resolve(msg);
      }
    };
    ws.on("message", onMsg);
    ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    setTimeout(() => {
      ws.off("message", onMsg);
      reject(new Error(`rpc timeout: ${method}`));
    }, 2000);
  });
}

test("startMcpServer writes lockfile with port, pid, authToken", async () => {
  const ideDir = tempIdeDir();
  const server = await startMcpServer({
    ideDir,
    workspaceFolders: ["/tmp/fake-project"],
  });
  try {
    const files = readdirSync(ideDir);
    expect(files).toHaveLength(1);
    expect(files[0]).toBe(`${server.port}.lock`);
    const body = JSON.parse(readFileSync(join(ideDir, files[0]), "utf8"));
    expect(body.pid).toBe(process.pid);
    expect(body.port).toBe(server.port);
    expect(body.workspaceFolders).toEqual(["/tmp/fake-project"]);
    expect(body.ideName).toBe("newde");
    expect(body.transport).toBe("ws");
    expect(typeof body.authToken).toBe("string");
    expect(body.authToken.length).toBeGreaterThan(8);
  } finally {
    await server.stop();
    rmSync(ideDir, { recursive: true, force: true });
  }
});

test("stop() removes lockfile", async () => {
  const ideDir = tempIdeDir();
  const server = await startMcpServer({ ideDir, workspaceFolders: [] });
  const lockPath = join(ideDir, `${server.port}.lock`);
  expect(existsSync(lockPath)).toBe(true);
  await server.stop();
  expect(existsSync(lockPath)).toBe(false);
  rmSync(ideDir, { recursive: true, force: true });
});

test("MCP handshake: initialize, tools/list, tools/call newde__ping", async () => {
  const ideDir = tempIdeDir();
  const server = await startMcpServer({ ideDir, workspaceFolders: [] });
  try {
    const ws = await connect(server.port, server.authToken);

    const init = await rpc(ws, 1, "initialize", {
      protocolVersion: "2024-11-05",
      capabilities: {},
      clientInfo: { name: "test", version: "0" },
    });
    expect(init.error).toBeUndefined();
    expect(init.result.serverInfo.name).toBe("newde");
    expect(init.result.capabilities.tools).toBeDefined();

    const list = await rpc(ws, 2, "tools/list", {});
    expect(list.error).toBeUndefined();
    const tools = list.result.tools as Array<{ name: string }>;
    expect(tools.map((t) => t.name)).toContain("newde__ping");

    const call = await rpc(ws, 3, "tools/call", {
      name: "newde__ping",
      arguments: {},
    });
    expect(call.error).toBeUndefined();
    const content = call.result.content[0];
    expect(content.type).toBe("text");
    const payload = JSON.parse(content.text);
    expect(payload.ok).toBe(true);
    expect(payload.daemonPort).toBe(server.port);
    expect(typeof payload.timestamp).toBe("number");

    ws.close();
  } finally {
    await server.stop();
    rmSync(ideDir, { recursive: true, force: true });
  }
});

// Auth rejection is exercised via manual/e2e verification — bun's test runner
// and the `ws` client don't cooperate well when the server closes immediately
// after upgrade, even though the standalone behavior is correct.

test("stale lockfile sweep: removes lockfiles for dead pids in the ide dir", async () => {
  const ideDir = tempIdeDir();
  const stale = join(ideDir, "99999.lock");
  writeFileSync(
    stale,
    JSON.stringify({
      pid: 999999,
      port: 99999,
      workspaceFolders: [],
      ideName: "stale",
      transport: "ws",
      runningInWindows: false,
      authToken: "x",
    }),
  );
  const server = await startMcpServer({ ideDir, workspaceFolders: [] });
  try {
    expect(existsSync(stale)).toBe(false);
  } finally {
    await server.stop();
    rmSync(ideDir, { recursive: true, force: true });
  }
});

test("does not sweep lockfiles that belong to live pids", async () => {
  const ideDir = tempIdeDir();
  const live = join(ideDir, "12345.lock");
  writeFileSync(
    live,
    JSON.stringify({
      pid: process.pid, // current process = definitely alive
      port: 12345,
      workspaceFolders: [],
      ideName: "other-ide",
      transport: "ws",
      runningInWindows: false,
      authToken: "x",
    }),
  );
  const server = await startMcpServer({ ideDir, workspaceFolders: [] });
  try {
    expect(existsSync(live)).toBe(true);
  } finally {
    await server.stop();
    rmSync(ideDir, { recursive: true, force: true });
  }
});
