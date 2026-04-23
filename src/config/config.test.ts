import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { OXPLOW_CONFIG_FILE, loadProjectConfig, parseOxplowConfig } from "./config.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    rmSync(tempDirs.pop()!, { recursive: true, force: true });
  }
});

test("loadProjectConfig defaults agent=claude and projectName=basename when oxplow.yaml is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-config-"));
  tempDirs.push(dir);
  const config = loadProjectConfig(dir);
  expect(config.agent).toBe("claude");
  expect(config.projectName).toBe(dir.split("/").pop());
});

test("loadProjectConfig reads the configured agent from oxplow.yaml", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, OXPLOW_CONFIG_FILE), "agent: copilot\n", "utf8");
  expect(loadProjectConfig(dir).agent).toBe("copilot");
});

test("loadProjectConfig reads an explicit projectName from oxplow.yaml", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, OXPLOW_CONFIG_FILE), "projectName: My Fancy Project\n", "utf8");
  expect(loadProjectConfig(dir).projectName).toBe("My Fancy Project");
});

test("loadProjectConfig falls back to basename when projectName is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, OXPLOW_CONFIG_FILE), "agent: claude\n", "utf8");
  expect(loadProjectConfig(dir).projectName).toBe(dir.split("/").pop());
});

test("parseOxplowConfig rejects invalid agent values", () => {
  expect(() => parseOxplowConfig({ agent: "cursor" })).toThrow(
    "oxplow.yaml agent must be either 'claude' or 'copilot'",
  );
});

test("parseOxplowConfig rejects unknown keys", () => {
  expect(() => parseOxplowConfig({ agent: "claude", theme: "dark" })).toThrow(
    "oxplow.yaml contains unknown key: theme",
  );
});

test("parseOxplowConfig rejects empty projectName", () => {
  expect(() => parseOxplowConfig({ projectName: "   " })).toThrow(
    "oxplow.yaml projectName must be a non-empty string",
  );
});

test("parseOxplowConfig parses lsp.servers entries", () => {
  const parsed = parseOxplowConfig({
    lsp: {
      servers: [
        { languageId: "python", extensions: [".py"], command: "pyright-langserver", args: ["--stdio"] },
        { languageId: "rust", extensions: [".rs"], command: "rust-analyzer" },
      ],
    },
  });
  expect(parsed.lspServers).toEqual([
    { languageId: "python", extensions: [".py"], command: "pyright-langserver", args: ["--stdio"] },
    { languageId: "rust", extensions: [".rs"], command: "rust-analyzer", args: [] },
  ]);
});

test("parseOxplowConfig rejects lsp entries missing required fields", () => {
  expect(() =>
    parseOxplowConfig({
      lsp: { servers: [{ languageId: "python", extensions: [".py"] }] },
    }),
  ).toThrow(/command/);
  expect(() =>
    parseOxplowConfig({
      lsp: { servers: [{ languageId: "python", command: "pyls" }] },
    }),
  ).toThrow(/extensions/);
  expect(() =>
    parseOxplowConfig({
      lsp: { servers: [{ extensions: [".py"], command: "pyls" }] },
    }),
  ).toThrow(/languageId/);
});

test("parseOxplowConfig rejects lsp extensions that don't start with a dot", () => {
  expect(() =>
    parseOxplowConfig({
      lsp: { servers: [{ languageId: "python", extensions: ["py"], command: "pyls" }] },
    }),
  ).toThrow(/extension/);
});

test("injectSessionContext defaults to true and round-trips", () => {
  expect(parseOxplowConfig({}).injectSessionContext).toBe(true);
  expect(parseOxplowConfig({ injectSessionContext: false }).injectSessionContext).toBe(false);
  expect(() => parseOxplowConfig({ injectSessionContext: "yes" })).toThrow(/injectSessionContext/);
});
