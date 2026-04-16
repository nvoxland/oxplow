import { expect, test } from "bun:test";
import { buildBatchMcpConfig } from "./runtime.js";
import type { McpServerHandle } from "../mcp-server.js";

function fakeMcp(overrides: Partial<McpServerHandle> = {}): McpServerHandle {
  return {
    port: 43123,
    authToken: "secret-token",
    httpUrl: "http://127.0.0.1:43123/mcp",
    lockfilePath: "/tmp/43123.lock",
    stop: async () => {},
    ...overrides,
  };
}

test("buildBatchMcpConfig points Claude at the shared HTTP MCP endpoint", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp()));
  expect(config.mcpServers.newde).toEqual({
    type: "http",
    url: "http://127.0.0.1:43123/mcp",
    headers: {
      Authorization: "Bearer secret-token",
    },
  });
});

test("buildBatchMcpConfig only declares the newde server", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp()));
  expect(Object.keys(config.mcpServers)).toEqual(["newde"]);
});

test("buildBatchMcpConfig embeds the exact bearer format", () => {
  const config = JSON.parse(buildBatchMcpConfig(fakeMcp({ authToken: "abc.def-ghi" })));
  expect(config.mcpServers.newde.headers.Authorization).toBe("Bearer abc.def-ghi");
});

test("buildBatchMcpConfig throws when the MCP server is not running", () => {
  expect(() => buildBatchMcpConfig(null)).toThrow("mcp server not started");
});
