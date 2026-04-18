import { expect, test } from "bun:test";
import { mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createDaemonLogger, createUiClientLogger } from "./logger.js";

function readJsonLines(path: string): any[] {
  return readFileSync(path, "utf8")
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

test("createDaemonLogger writes structured logs to the project daemon log", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-logger-test-"));
  const logger = createDaemonLogger(projectDir).child({ subsystem: "daemon-test" });

  logger.info("daemon started", { port: 7457 });

  expect(logger.path).toBe(join(projectDir, ".newde", "logs", "system.log"));
  const lines = readJsonLines(logger.path);
  expect(lines).toHaveLength(1);
  expect(lines[0].level).toBe("info");
  expect(lines[0].message).toBe("daemon started");
  expect(lines[0].context).toEqual({ subsystem: "daemon-test", port: 7457 });
  expect(typeof lines[0].time).toBe("string");
});

test("child loggers merge context across writes", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-logger-test-"));
  const logger = createDaemonLogger(projectDir).child({ streamId: "s-1" }).child({ pane: "working" });

  logger.warn("pane resumed", { sessionId: "abc" });

  const [line] = readJsonLines(logger.path);
  expect(line.context).toEqual({ streamId: "s-1", pane: "working", sessionId: "abc" });
});

test("createUiClientLogger writes to a per-client log file with a safe file name", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-logger-test-"));
  const logger = createUiClientLogger(projectDir, "browser/tab:1");

  logger.error("ui crash", { route: "/streams" });

  expect(logger.path).toBe(join(projectDir, ".newde", "logs", "ui", "browser_tab_1.log"));
  const [line] = readJsonLines(logger.path);
  expect(line.level).toBe("error");
  expect(line.message).toBe("ui crash");
  expect(line.context).toEqual({ clientId: "browser/tab:1", route: "/streams" });
});

test("createDaemonLogger mirrors info+ messages to the terminal", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-logger-test-"));
  const stdoutWrites: string[] = [];
  const stderrWrites: string[] = [];
  const originalStdout = process.stdout.write.bind(process.stdout);
  const originalStderr = process.stderr.write.bind(process.stderr);
  process.stdout.write = ((chunk: any, ...args: any[]) => {
    stdoutWrites.push(String(chunk));
    return true;
  }) as typeof process.stdout.write;
  process.stderr.write = ((chunk: any, ...args: any[]) => {
    stderrWrites.push(String(chunk));
    return true;
  }) as typeof process.stderr.write;

  try {
    const logger = createDaemonLogger(projectDir).child({ subsystem: "daemon-test" });
    logger.debug("debug only");
    logger.info("daemon started", { port: 7457 });
    logger.warn("watch out");
    logger.error("boom");
  } finally {
    process.stdout.write = originalStdout;
    process.stderr.write = originalStderr;
  }

  expect(stdoutWrites.join("")).toContain("INFO daemon started");
  expect(stdoutWrites.join("")).toContain("WARN watch out");
  expect(stdoutWrites.join("")).not.toContain("debug only");
  expect(stderrWrites.join("")).toContain("ERROR boom");
});

test("createUiClientLogger does not mirror to the terminal", () => {
  const projectDir = mkdtempSync(join(tmpdir(), "newde-logger-test-"));
  const stdoutWrites: string[] = [];
  const stderrWrites: string[] = [];
  const originalStdout = process.stdout.write.bind(process.stdout);
  const originalStderr = process.stderr.write.bind(process.stderr);
  process.stdout.write = ((chunk: any, ...args: any[]) => {
    stdoutWrites.push(String(chunk));
    return true;
  }) as typeof process.stdout.write;
  process.stderr.write = ((chunk: any, ...args: any[]) => {
    stderrWrites.push(String(chunk));
    return true;
  }) as typeof process.stderr.write;

  try {
    createUiClientLogger(projectDir, "client-1").info("browser log");
  } finally {
    process.stdout.write = originalStdout;
    process.stderr.write = originalStderr;
  }

  expect(stdoutWrites).toEqual([]);
  expect(stderrWrites).toEqual([]);
});
