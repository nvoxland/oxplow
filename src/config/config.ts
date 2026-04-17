import { existsSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import YAML from "yaml";
import type { Logger } from "../core/logger.js";

export const NEWDE_CONFIG_FILE = "newde.yaml";

export type AgentKind = "claude" | "copilot";

export interface NewdeConfig {
  agent: AgentKind;
  /** Human-readable project name. Defaults to the basename of projectDir when
   *  not set in newde.yaml. */
  projectName: string;
}

/** Partial shape that survives YAML parsing — loadProjectConfig fills in
 *  env-dependent defaults (like projectName = basename(projectDir)). */
export interface ParsedNewdeConfig {
  agent: AgentKind;
  projectName?: string;
}

const DEFAULT_AGENT: AgentKind = "claude";

export function loadProjectConfig(projectDir: string, logger?: Logger): NewdeConfig {
  const configPath = join(projectDir, NEWDE_CONFIG_FILE);
  const fallbackName = basename(resolve(projectDir));
  if (!existsSync(configPath)) {
    logger?.info("project config not found; using defaults", { configPath, agent: DEFAULT_AGENT });
    return { agent: DEFAULT_AGENT, projectName: fallbackName };
  }

  const raw = readFileSync(configPath, "utf8");
  const parsed = parseNewdeConfig(YAML.parse(raw));
  const config: NewdeConfig = {
    agent: parsed.agent,
    projectName: parsed.projectName ?? fallbackName,
  };
  logger?.info("loaded project config", { configPath, agent: config.agent, projectName: config.projectName });
  return config;
}

export function parseNewdeConfig(value: unknown): ParsedNewdeConfig {
  if (!isRecord(value)) {
    throw new Error("newde.yaml must contain a YAML object");
  }

  const allowedKeys = new Set(["agent", "projectName"]);
  for (const key of Object.keys(value)) {
    if (!allowedKeys.has(key)) {
      throw new Error(`newde.yaml contains unknown key: ${key}`);
    }
  }

  const rawAgent = value.agent;
  let agent: AgentKind = DEFAULT_AGENT;
  if (rawAgent !== undefined) {
    if (rawAgent !== "claude" && rawAgent !== "copilot") {
      throw new Error("newde.yaml agent must be either 'claude' or 'copilot'");
    }
    agent = rawAgent;
  }

  let projectName: string | undefined;
  if (value.projectName !== undefined) {
    if (typeof value.projectName !== "string" || value.projectName.trim() === "") {
      throw new Error("newde.yaml projectName must be a non-empty string");
    }
    projectName = value.projectName.trim();
  }

  return { agent, projectName };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
