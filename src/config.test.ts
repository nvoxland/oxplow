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

test("loadProjectConfig defaults to Claude when newde.yaml is absent", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  expect(loadProjectConfig(dir)).toEqual({ agent: "claude" });
});

test("loadProjectConfig reads the configured agent from newde.yaml", () => {
  const dir = mkdtempSync(join(tmpdir(), "newde-config-"));
  tempDirs.push(dir);
  writeFileSync(join(dir, NEWDE_CONFIG_FILE), "agent: copilot\n", "utf8");
  expect(loadProjectConfig(dir)).toEqual({ agent: "copilot" });
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
