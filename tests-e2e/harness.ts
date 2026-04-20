import { _electron as electron, type ElectronApplication, type Page } from "playwright";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync, existsSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));

export interface LaunchedNewde {
  app: ElectronApplication;
  window: Page;
  close: () => Promise<void>;
}

// Launch the built newde Electron app, pointed at `projectDir`.
// Mirrors what `bin/newde` does: `electron . --project <dir>`.
//
// Each launch gets a fresh Electron userData directory via
// --user-data-dir. Without this, localStorage (dock open/collapsed,
// open file tabs, etc.) persists across probe runs and one probe's
// UI state corrupts the next — e.g. a probe that clicks the active
// "Files" dock tab toggles the dock closed, and the next probe finds
// file-tree rows in the DOM but display:none. Isolating userData
// per launch is what makes probes reproducible.
export async function launchNewde(projectDir: string, opts: { timeoutMs?: number } = {}): Promise<LaunchedNewde> {
  const repoRoot = resolve(__dirname, "..");
  const userDataDir = mkdtempSync(join(tmpdir(), "newde-e2e-userdata-"));

  const app = await electron.launch({
    args: [repoRoot, `--user-data-dir=${userDataDir}`, "--project", projectDir],
    cwd: repoRoot,
    timeout: opts.timeoutMs ?? 60_000,
    env: {
      ...process.env,
      ELECTRON_DISABLE_SECURITY_WARNINGS: "1",
    },
  });

  // Forward Electron stdout/stderr so launch failures are debuggable.
  app.process().stdout?.on("data", (b) => process.stdout.write(`[electron:out] ${b}`));
  app.process().stderr?.on("data", (b) => process.stderr.write(`[electron:err] ${b}`));

  const window = await app.firstWindow({ timeout: opts.timeoutMs ?? 60_000 });
  await window.waitForLoadState("domcontentloaded");

  return {
    app,
    window,
    close: async () => {
      await app.close();
      try { rmSync(userDataDir, { recursive: true, force: true }); } catch { /* best-effort */ }
    },
  };
}

// Trigger a native-menu-backed command by its CommandId — same IPC channel
// the Electron menubar uses. Playwright's `window.keyboard.press("Meta+K")`
// only works for shortcuts wired through a window-level keydown listener
// (e.g. Cmd+K). Everything else (Cmd+P quick-open, Cmd+S save, file.find,
// etc.) routes through the native menu, which Playwright can't click. Use
// this helper for those.
export async function sendMenuCommand(app: ElectronApplication, commandId: string): Promise<void> {
  await app.evaluate(({ BrowserWindow }, id) => {
    const win = BrowserWindow.getAllWindows()[0];
    win?.webContents.send("newde:menu-command", id);
  }, commandId);
}

// Force stdout through the buffer, so heartbeats and progress lines are
// actually visible while the probe is running. Node's console.log is
// block-buffered when stdout is a pipe/file; without flushing, probe
// output only appears after the process exits, which is catastrophic when
// a probe hangs. Call after anything you'd want visible mid-run.
export function probeLog(message: string): void {
  // eslint-disable-next-line no-console
  console.log(message);
  // process.stdout.write forces a drain attempt; with a subsequent
  // microtask yield the line usually reaches the pipe immediately.
  try { (process.stdout as any)._flush?.(); } catch { /* ignore */ }
}

// Wrap a probe's main() with (a) a preflight that kills stray probe
// electrons and stale instance locks, (b) a wall-clock hard timeout, and
// (c) a silence watchdog that fails fast if the probe hasn't emitted a
// `[probe]` line in `silenceMs`. Every probe should call this instead of
// invoking its own main() directly — that's how we keep hangs from
// eating 20 minutes of agent time.
export async function runProbe(
  name: string,
  fn: () => Promise<void>,
  opts: { wallMs?: number; silenceMs?: number } = {},
): Promise<void> {
  const wallMs = opts.wallMs ?? 90_000;
  const silenceMs = opts.silenceMs ?? 30_000;

  probeLog(`[probe:boot] ${name} wallMs=${wallMs} silenceMs=${silenceMs}`);
  preflightKillStrays();

  let lastTick = Date.now();
  const origWrite = process.stdout.write.bind(process.stdout);
  (process.stdout as any).write = (chunk: any, ...rest: any[]) => {
    lastTick = Date.now();
    return origWrite(chunk, ...rest);
  };

  const wall = setTimeout(() => {
    probeLog(`[probe:fail] ${name} wall-clock exceeded ${wallMs}ms — exiting`);
    process.exit(124);
  }, wallMs);
  const watchdog = setInterval(() => {
    if (Date.now() - lastTick > silenceMs) {
      probeLog(`[probe:fail] ${name} silent for ${silenceMs}ms — exiting`);
      process.exit(125);
    }
  }, Math.max(1000, Math.floor(silenceMs / 4)));

  try {
    await fn();
    probeLog(`[probe:done] ${name}`);
  } catch (err) {
    probeLog(`[probe:fail] ${name} threw: ${String(err)}`);
    process.exitCode = 1;
    throw err;
  } finally {
    clearTimeout(wall);
    clearInterval(watchdog);
  }
}

// Kill any probe-spawned electron processes or orphaned user-data dirs
// from prior aborted runs. Matches on our launch args (--user-data-dir
// prefix `newde-e2e-userdata-`) so we don't touch the user's other
// Electron apps. Also clears a stale instance lock if the PID inside is
// dead.
export function preflightKillStrays(): void {
  try {
    const pids = execSync(
      "pgrep -f 'newde-e2e-userdata-' 2>/dev/null || true",
      { encoding: "utf8" },
    ).trim().split(/\s+/).filter(Boolean);
    if (pids.length > 0) {
      probeLog(`[probe:boot] killing ${pids.length} stray electron pid(s): ${pids.join(",")}`);
      try { execSync(`kill -9 ${pids.join(" ")} 2>/dev/null || true`); } catch { /* ignore */ }
    }
  } catch { /* ignore */ }

  const lock = resolve(__dirname, "..", ".newde", "runtime", "instance.lock");
  if (existsSync(lock)) {
    try {
      const pid = Number(readFileSync(lock, "utf8").trim());
      if (pid && !isPidAlive(pid)) {
        probeLog(`[probe:boot] removing stale instance.lock for dead pid ${pid}`);
        rmSync(lock, { force: true });
      }
    } catch { /* ignore */ }
  }
}

function isPidAlive(pid: number): boolean {
  try { process.kill(pid, 0); return true; } catch { return false; }
}
