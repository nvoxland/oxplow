import { existsSync, readFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import YAML from "yaml";
import type { Logger } from "../core/logger.js";

export const NEWDE_CONFIG_FILE = "newde.yaml";

export type AgentKind = "claude" | "copilot";

export interface NewdeLspServerConfig {
  languageId: string;
  extensions: string[];
  command: string;
  args: string[];
}

export interface NewdeConfig {
  agent: AgentKind;
  /** Human-readable project name. Defaults to the basename of projectDir when
   *  not set in newde.yaml. */
  projectName: string;
  /** Extra language servers to register on top of the built-ins. */
  lspServers: NewdeLspServerConfig[];
}

/** Partial shape that survives YAML parsing — loadProjectConfig fills in
 *  env-dependent defaults (like projectName = basename(projectDir)). */
export interface ParsedNewdeConfig {
  agent: AgentKind;
  projectName?: string;
  lspServers: NewdeLspServerConfig[];
}

const DEFAULT_AGENT: AgentKind = "claude";

export function loadProjectConfig(projectDir: string, logger?: Logger): NewdeConfig {
  const configPath = join(projectDir, NEWDE_CONFIG_FILE);
  const fallbackName = basename(resolve(projectDir));
  if (!existsSync(configPath)) {
    logger?.info("project config not found; using defaults", { configPath, agent: DEFAULT_AGENT });
    return { agent: DEFAULT_AGENT, projectName: fallbackName, lspServers: [] };
  }

  const raw = readFileSync(configPath, "utf8");
  const parsed = parseNewdeConfig(YAML.parse(raw));
  const config: NewdeConfig = {
    agent: parsed.agent,
    projectName: parsed.projectName ?? fallbackName,
    lspServers: parsed.lspServers,
  };
  logger?.info("loaded project config", {
    configPath,
    agent: config.agent,
    projectName: config.projectName,
    lspServers: config.lspServers.length,
  });
  return config;
}

export function parseNewdeConfig(value: unknown): ParsedNewdeConfig {
  if (!isRecord(value)) {
    throw new Error("newde.yaml must contain a YAML object");
  }

  const allowedKeys = new Set(["agent", "projectName", "lsp"]);
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

  return { agent, projectName, lspServers: parseLspServers(value.lsp) };
}

function parseLspServers(rawLsp: unknown): NewdeLspServerConfig[] {
  if (rawLsp === undefined) return [];
  if (!isRecord(rawLsp)) {
    throw new Error("newde.yaml lsp must be an object");
  }
  for (const key of Object.keys(rawLsp)) {
    if (key !== "servers") throw new Error(`newde.yaml lsp contains unknown key: ${key}`);
  }
  const rawServers = rawLsp.servers;
  if (rawServers === undefined) return [];
  if (!Array.isArray(rawServers)) {
    throw new Error("newde.yaml lsp.servers must be an array");
  }
  return rawServers.map((entry, index) => parseLspServerEntry(entry, index));
}

function parseLspServerEntry(entry: unknown, index: number): NewdeLspServerConfig {
  if (!isRecord(entry)) {
    throw new Error(`newde.yaml lsp.servers[${index}] must be an object`);
  }
  const languageId = entry.languageId;
  if (typeof languageId !== "string" || languageId.trim() === "") {
    throw new Error(`newde.yaml lsp.servers[${index}].languageId must be a non-empty string`);
  }
  const command = entry.command;
  if (typeof command !== "string" || command.trim() === "") {
    throw new Error(`newde.yaml lsp.servers[${index}].command must be a non-empty string`);
  }
  const rawExtensions = entry.extensions;
  if (!Array.isArray(rawExtensions) || rawExtensions.length === 0) {
    throw new Error(`newde.yaml lsp.servers[${index}].extensions must be a non-empty array`);
  }
  const extensions = rawExtensions.map((extension, extIndex) => {
    if (typeof extension !== "string" || !extension.startsWith(".")) {
      throw new Error(
        `newde.yaml lsp.servers[${index}].extensions[${extIndex}] must be a string starting with '.'`,
      );
    }
    return extension.toLowerCase();
  });
  let args: string[] = [];
  if (entry.args !== undefined) {
    if (!Array.isArray(entry.args) || entry.args.some((arg) => typeof arg !== "string")) {
      throw new Error(`newde.yaml lsp.servers[${index}].args must be an array of strings`);
    }
    args = entry.args as string[];
  }
  return { languageId, extensions, command, args };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
