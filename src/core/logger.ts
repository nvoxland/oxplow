import { appendFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { ensureOxplowRoot } from "./oxplow-dir.js";

export type LogLevel = "debug" | "info" | "warn" | "error";

export interface Logger {
  readonly path: string;
  debug(message: string, context?: Record<string, unknown>): void;
  info(message: string, context?: Record<string, unknown>): void;
  warn(message: string, context?: Record<string, unknown>): void;
  error(message: string, context?: Record<string, unknown>): void;
  child(context: Record<string, unknown>): Logger;
}

interface LogEntry {
  time: string;
  level: LogLevel;
  message: string;
  context?: Record<string, unknown>;
}

interface ConsoleOptions {
  minLevel: LogLevel;
}

class FileLogger implements Logger {
  readonly path: string;
  private readonly baseContext: Record<string, unknown>;
  private readonly consoleOptions?: ConsoleOptions;

  constructor(path: string, baseContext: Record<string, unknown> = {}, consoleOptions?: ConsoleOptions) {
    this.path = path;
    this.baseContext = baseContext;
    this.consoleOptions = consoleOptions;
    mkdirSync(dirname(path), { recursive: true });
  }

  debug(message: string, context?: Record<string, unknown>): void {
    this.write("debug", message, context);
  }

  info(message: string, context?: Record<string, unknown>): void {
    this.write("info", message, context);
  }

  warn(message: string, context?: Record<string, unknown>): void {
    this.write("warn", message, context);
  }

  error(message: string, context?: Record<string, unknown>): void {
    this.write("error", message, context);
  }

  child(context: Record<string, unknown>): Logger {
    return new FileLogger(this.path, { ...this.baseContext, ...context }, this.consoleOptions);
  }

  private write(level: LogLevel, message: string, context?: Record<string, unknown>) {
    const mergedContext = { ...this.baseContext, ...(context ?? {}) };
    const entry: LogEntry = {
      time: new Date().toISOString(),
      level,
      message,
      ...(Object.keys(mergedContext).length > 0 ? { context: mergedContext } : {}),
    };
    appendFileSync(this.path, JSON.stringify(entry) + "\n", "utf8");
    this.writeConsole(entry);
  }

  private writeConsole(entry: LogEntry) {
    if (!this.consoleOptions) return;
    if (levelRank(entry.level) < levelRank(this.consoleOptions.minLevel)) return;
    const line = formatConsoleEntry(entry);
    if (entry.level === "error") {
      process.stderr.write(line + "\n");
      return;
    }
    process.stdout.write(line + "\n");
  }
}

export function createDaemonLogger(projectDir: string): Logger {
  ensureOxplowRoot(projectDir);
  return new FileLogger(join(projectDir, ".oxplow", "logs", "system.log"), {}, { minLevel: "info" });
}

export function createUiClientLogger(projectDir: string, clientId: string): Logger {
  ensureOxplowRoot(projectDir);
  return new FileLogger(join(projectDir, ".oxplow", "logs", "ui", `${sanitizeLogFileSegment(clientId)}.log`), { clientId });
}

function sanitizeLogFileSegment(value: string): string {
  return value.replace(/[^a-zA-Z0-9._-]/g, "_") || "unknown-client";
}

function levelRank(level: LogLevel): number {
  switch (level) {
    case "debug": return 10;
    case "info": return 20;
    case "warn": return 30;
    case "error": return 40;
  }
}

function formatConsoleEntry(entry: LogEntry): string {
  const context = entry.context ? ` ${JSON.stringify(entry.context)}` : "";
  return `[oxplow] ${entry.level.toUpperCase()} ${entry.message}${context}`;
}
