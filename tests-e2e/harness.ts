import { _electron as electron, type ElectronApplication, type Page } from "playwright";
import { resolve } from "node:path";

export interface LaunchedNewde {
  app: ElectronApplication;
  window: Page;
  close: () => Promise<void>;
}

// Launch the built newde Electron app, pointed at `projectDir`.
// Mirrors what `bin/newde` does: `electron . --project <dir>`.
export async function launchNewde(projectDir: string, opts: { timeoutMs?: number } = {}): Promise<LaunchedNewde> {
  const repoRoot = resolve(__dirname, "..");
  const electronEntry = resolve(repoRoot, "dist/electron-main.cjs");

  const app = await electron.launch({
    args: [repoRoot, "--project", projectDir],
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
    },
  };
}
