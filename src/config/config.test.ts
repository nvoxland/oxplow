import { afterEach, expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { NEWDE_CONFIG_FILE, loadProjectConfig, parseNewdeConfig } from "./config.js";

const tempDirs: string[] = [];

afterEach(() => {
  while (tempDirs.length > 0) {
    rmSync(tempDirs.pop()!, { recursive: true, force: true });
  }
});

test("loadProjectConfig defaults agent=claude and projectName=basename when newde.yaml is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  const config = loadProjectConfig(dir);
  expect(config.agent).toBe("claude");
  expect(config.projectName).toBe(dir.split("/").pop());
});

test("loadProjectConfig reads the configured agent from newde.yaml", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, NEWDE_CONFIG_FILE), "agent: copilot\n", "utf8");
  expect(loadProjectConfig(dir).agent).toBe("copilot");
});

test("loadProjectConfig reads an explicit projectName from newde.yaml", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, NEWDE_CONFIG_FILE), "projectName: My Fancy Project\n", "utf8");
  expect(loadProjectConfig(dir).projectName).toBe("My Fancy Project");
});

test("loadProjectConfig falls back to basename when projectName is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, NEWDE_CONFIG_FILE), "agent: claude\n", "utf8");
  expect(loadProjectConfig(dir).projectName).toBe(dir.split("/").pop());
});

test("parseNewdeConfig rejects invalid agent values", () => {
  expect(() => parseNewdeConfig({ agent: "cursor" })).toThrow(
    "newde.yaml agent must be either 'claude' or 'copilot'",
  );
});

test("parseNewdeConfig rejects unknown keys", () => {
  expect(() => parseNewdeConfig({ agent: "claude", theme: "dark" })).toThrow(
    "newde.yaml contains unknown key: theme",
  );
});

test("parseNewdeConfig rejects empty projectName", () => {
  expect(() => parseNewdeConfig({ projectName: "   " })).toThrow(
    "newde.yaml projectName must be a non-empty string",
  );
});

test("parseNewdeConfig parses lsp.servers entries", () => {
  const parsed = parseNewdeConfig({
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

test("parseNewdeConfig rejects lsp entries missing required fields", () => {
  expect(() =>
    parseNewdeConfig({
      lsp: { servers: [{ languageId: "python", extensions: [".py"] }] },
    }),
  ).toThrow(/command/);
  expect(() =>
    parseNewdeConfig({
      lsp: { servers: [{ languageId: "python", command: "pyls" }] },
    }),
  ).toThrow(/extensions/);
  expect(() =>
    parseNewdeConfig({
      lsp: { servers: [{ extensions: [".py"], command: "pyls" }] },
    }),
  ).toThrow(/languageId/);
});

test("parseNewdeConfig rejects lsp extensions that don't start with a dot", () => {
  expect(() =>
    parseNewdeConfig({
      lsp: { servers: [{ languageId: "python", extensions: ["py"], command: "pyls" }] },
    }),
  ).toThrow(/extension/);
});

test("injectSessionContext defaults to true and round-trips", () => {
  expect(parseNewdeConfig({}).injectSessionContext).toBe(true);
  expect(parseNewdeConfig({ injectSessionContext: false }).injectSessionContext).toBe(false);
  expect(() => parseNewdeConfig({ injectSessionContext: "yes" })).toThrow(/injectSessionContext/);
});
