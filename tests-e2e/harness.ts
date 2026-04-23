import { _electron as electron, type ElectronApplication, type Page } from "playwright";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execSync } from "node:child_process";

const __dirname = dirname(fileURLToPath(import.meta.url));

export interface LaunchedOxplow {
  app: ElectronApplication;
  window: Page;
  close: () => Promise<void>;
}

// Launch the built oxplow Electron app, pointed at `projectDir`.
// Mirrors what `bin/oxplow` does: `electron . --project <dir>`.
//
// Each launch gets a fresh Electron userData directory via
// --user-data-dir. Without this, localStorage (dock open/collapsed,
// open file tabs, etc.) persists across probe runs and one probe's
// UI state corrupts the next — e.g. a probe that clicks the active
// "Files" dock tab toggles the dock closed, and the next probe finds
// file-tree rows in the DOM but display:none. Isolating userData
// per launch is what makes probes reproducible.
export async function launchOxplow(projectDir: string, opts: { timeoutMs?: number; fresh?: boolean } = {}): Promise<LaunchedOxplow> {
  const repoRoot = resolve(__dirname, "..");
  const userDataDir = mkdtempSync(join(tmpdir(), "oxplow-e2e-userdata-"));

  // When `fresh: true`, wipe the project's persisted oxplow state so a
  // probe doesn't inherit stale work_items / commit_points / wait_points
  // from a prior run. Without this, the Stop hook can pick up an
  // orphaned work item from a totally different session and steer the
  // inner agent sideways. Targets `.oxplow/state.sqlite*` (DB + WAL +
  // SHM) and the runtime instance lock; leaves snapshots/git alone.
  if (opts.fresh) {
    for (const name of ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"]) {
      try { rmSync(join(projectDir, ".oxplow", name), { force: true }); } catch { /* ignore */ }
    }
    try { rmSync(join(projectDir, ".oxplow", "runtime", "instance.lock"), { force: true }); } catch { /* ignore */ }
  }

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

/**
 * Wait until oxplow's chrome has mounted by polling for the dock-tab-plan
 * testid. Replaces the blind `await window.waitForTimeout(3_000)` that
 * every probe used to sleep for at startup. Returns as soon as the
 * marker is present (typically <1s on a warm machine), or throws if it
 * doesn't show up within `timeoutMs`.
 */
export async function waitForOxplowReady(window: Page, opts: { timeoutMs?: number } = {}): Promise<void> {
  const timeoutMs = opts.timeoutMs ?? 15_000;
  await window.locator('[data-testid="dock-tab-plan"]').first().waitFor({ state: "visible", timeout: timeoutMs });
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
    win?.webContents.send("oxplow:menu-command", id);
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
// prefix `oxplow-e2e-userdata-`) so we don't touch the user's other
// Electron apps. Also clears a stale instance lock if the PID inside is
// dead.
export function preflightKillStrays(): void {
  try {
    const pids = execSync(
      "pgrep -f 'oxplow-e2e-userdata-' 2>/dev/null || true",
      { encoding: "utf8" },
    ).trim().split(/\s+/).filter(Boolean);
    if (pids.length > 0) {
      probeLog(`[probe:boot] killing ${pids.length} stray electron pid(s): ${pids.join(",")}`);
      try { execSync(`kill -9 ${pids.join(" ")} 2>/dev/null || true`); } catch { /* ignore */ }
    }
  } catch { /* ignore */ }

  const lock = resolve(__dirname, "..", ".oxplow", "runtime", "instance.lock");
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

// ---------------------------------------------------------------------
// Dogfood helpers
//
// The three helpers below collapse the pattern that every dogfood probe
// used to open-code: add a commit point, create a single work item,
// prompt the xterm, poll until the agent goes quiet, then later
// approve via the Files-commit dialog in a separate launch.
//
// Extracted 2026-04-19 after review-20260419-221646.md called out
// three near-duplicate dogfood-commit-*.ts files.
// ---------------------------------------------------------------------

/**
 * Drive the canonical dogfood pass inside an already-launched oxplow
 * window:
 *   1. Click + Commit when done so propose_commit has something to
 *      target.
 *   2. Create one work item (title + body).
 *   3. Focus the xterm and type `prompt` followed by Enter.
 *   4. Poll every `tickMs` ms (default 15s), screenshotting and
 *      dumping .xterm-rows to `outDir/<slug>-rows-NN.txt`. Emit a
 *      heartbeat each tick so runProbe's silence watchdog is happy.
 *   5. Return when the terminal content has been unchanged for
 *      `quietTicks` ticks (default 3) or when `maxTicks` (default 40)
 *      is reached — whichever first.
 *
 * The caller decides what to do after (usually close & reopen to
 * approve). Does NOT rely on scrollback-text matching for "propose"
 * or "approve" — that proved unreliable in pass 2026-04-19 22:09
 * because prior-session text leaks in.
 */
export async function dogfoodInnerAgent(
  window: Page,
  opts: {
    slug: string;
    outDir: string;
    workItemTitle: string;
    workItemBody: string;
    prompt: string;
    tickMs?: number;
    quietTicks?: number;
    maxTicks?: number;
    addCommitPoint?: boolean;
  },
): Promise<{ ticks: number; exitReason: "quiet" | "max" }> {
  const tickMs = opts.tickMs ?? 15_000;
  const quietTicks = opts.quietTicks ?? 3;
  const maxTicks = opts.maxTicks ?? 40;
  const addCommitPoint = opts.addCommitPoint ?? true;

  // Activate Work panel.
  const workPanel = window.getByTestId("dock-panel-plan");
  for (let i = 0; i < 3; i++) {
    if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
    await window.getByTestId("dock-tab-plan").click();
    await window.waitForTimeout(300);
  }
  probeLog(`[dogfood:${opts.slug}] Work panel active`);

  // Add commit point so propose_commit has a target. Without this,
  // the agent will narrate "no active commit point existed" and
  // leave the suggestion as a work-note only.
  if (addCommitPoint) {
    await window.getByTestId("plan-add-commit-point").click();
    await window.waitForTimeout(400);
    probeLog(`[dogfood:${opts.slug}] + Commit when done clicked`);
  }

  // Create one work item for Plan-UI visibility (the allowed
  // single-item exception to "don't pre-queue").
  await window.getByTestId("plan-new-task").click();
  await window.waitForTimeout(400);
  await window.getByTestId("work-item-title").fill(opts.workItemTitle);
  await window.getByTestId("work-item-description").fill(opts.workItemBody);
  await window.getByTestId("work-item-save").click();
  await window.waitForTimeout(800);
  probeLog(`[dogfood:${opts.slug}] work item created: ${opts.workItemTitle}`);

  // Focus xterm and send prompt.
  const xterm = window.locator(".xterm").first();
  await xterm.waitFor({ state: "visible", timeout: 5_000 });
  await xterm.click();
  await window.waitForTimeout(400);
  await window.keyboard.type(opts.prompt);
  await window.waitForTimeout(500);
  await window.keyboard.press("Enter");
  probeLog(`[dogfood:${opts.slug}] prompt sent`);

  // Poll for quiet.
  let tick = 0;
  let quiet = 0;
  let lastRows = "";
  while (tick < maxTicks) {
    tick += 1;
    await window.waitForTimeout(tickMs);
    const snapshotPath = resolve(opts.outDir, `${opts.slug}-poll-${String(tick).padStart(2, "0")}.png`);
    await window.screenshot({ path: snapshotPath });
    const rows = await window.evaluate(() => {
      const r = document.querySelector(".xterm-rows");
      return r ? (r as HTMLElement).innerText : "";
    });
    const rowsPath = resolve(opts.outDir, `${opts.slug}-rows-${String(tick).padStart(2, "0")}.txt`);
    try { writeFileSync(rowsPath, rows); } catch { /* best effort */ }
    if (rows === lastRows) quiet += 1; else quiet = 0;
    lastRows = rows;
    probeLog(`[dogfood:${opts.slug}] tick=${tick} quiet=${quiet}`);
    if (quiet >= quietTicks) {
      probeLog(`[dogfood:${opts.slug}] quiet for ${quietTicks} ticks; stopping`);
      return { ticks: tick, exitReason: "quiet" };
    }
  }
  probeLog(`[dogfood:${opts.slug}] hit maxTicks=${maxTicks}; stopping`);
  return { ticks: tick, exitReason: "max" };
}

/**
 * Synchronous `bun run build` — call between dogfood phase 1 and
 * phase 2 so the second launch picks up the inner agent's edits.
 * Without this, phase 2 runs against `dist/` from before the
 * inner agent's commit, so any UI-default change introduced in
 * the same pass won't take effect at approval time. See
 * `fix-20260419-225421-untracked-toggle.md`.
 */
export function runBuild(): void {
  const repoRoot = resolve(__dirname, "..");
  probeLog("[runBuild] bun run build");
  execSync("bun run build", { cwd: repoRoot, stdio: "ignore" });
  probeLog("[runBuild] done");
}

/**
 * Approve a pending set of changes by opening Files panel, clicking
 * the files-commit button, filling the message, and submitting.
 * Must be called inside an already-launched oxplow window.
 *
 * Since `096b2f0` the Files-commit dialog defaults Include-untracked
 * to OFF, so this helper only commits tracked changes. Pass an
 * `includeUntracked` option if you need the legacy `git add -A`
 * behavior.
 */
export async function approveViaFiles(
  window: Page,
  opts: { slug: string; outDir?: string; message: string; includeUntracked?: boolean },
): Promise<void> {
  await window.getByTestId("dock-tab-project").click();
  await window.waitForTimeout(400);
  await window.getByTestId("files-commit").click();
  await window.waitForTimeout(500);
  await window.getByTestId("files-commit-message").fill(opts.message);
  if (opts.includeUntracked) {
    const cb = window.locator("[data-testid='files-commit-include-untracked']");
    if (await cb.count() > 0) await cb.check();
  }
  if (opts.outDir) {
    await window.screenshot({ path: resolve(opts.outDir, `${opts.slug}-approve-filled.png`) });
  }
  await window.getByTestId("files-commit-submit").click();
  probeLog(`[approve:${opts.slug}] submitted`);
  await window.waitForTimeout(3_000);
  if (opts.outDir) {
    await window.screenshot({ path: resolve(opts.outDir, `${opts.slug}-approve-done.png`) });
  }
}
