import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import YAML from "yaml";
import type { Logger } from "./logger.js";

export const NEWDE_CONFIG_FILE = "newde.yaml";

export type AgentKind = "claude" | "copilot";

export interface NewdeConfig {
  agent: AgentKind;
}

const DEFAULT_CONFIG: NewdeConfig = {
  agent: "claude",
};

export function loadProjectConfig(projectDir: string, logger?: Logger): NewdeConfig {
  const configPath = join(projectDir, NEWDE_CONFIG_FILE);
  if (!existsSync(configPath)) {
    logger?.info("project config not found; using defaults", { configPath, agent: DEFAULT_CONFIG.agent });
    return { ...DEFAULT_CONFIG };
  }

  const raw = readFileSync(configPath, "utf8");
  const parsed = YAML.parse(raw);
  const config = parseNewdeConfig(parsed);
  logger?.info("loaded project config", { configPath, agent: config.agent });
  return config;
}

export function parseNewdeConfig(value: unknown): NewdeConfig {
  if (!isRecord(value)) {
    throw new Error("newde.yaml must contain a YAML object");
  }

  const allowedKeys = new Set(["agent"]);
  for (const key of Object.keys(value)) {
    if (!allowedKeys.has(key)) {
      throw new Error(`newde.yaml contains unknown key: ${key}`);
    }
  }

  const agent = value.agent;
  if (agent === undefined) {
    return { ...DEFAULT_CONFIG };
  }
  if (agent !== "claude" && agent !== "copilot") {
    throw new Error("newde.yaml agent must be either 'claude' or 'copilot'");
  }
  return { agent };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
