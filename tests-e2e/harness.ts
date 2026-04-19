import { _electron as electron, type ElectronApplication, type Page } from "playwright";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

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
