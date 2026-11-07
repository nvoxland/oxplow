// The browser suite (P11, `tests-e2e/README.md`): the built frontend, served
// by `vite preview`, against a real daemon — `oxplow-daemon-sim`, secrets in
// memory — over a throwaway git project, one per worker.
import { defineConfig, devices } from "@playwright/test";

import { previewPort } from "./tests-e2e/support/port.js";

// Each checkout's own, so worktrees' runs don't collide.
const PORT = previewPort(process.cwd(), process.env);
const PREVIEW = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "tests-e2e/specs",
  outputDir: "tests-e2e/.output/results",
  globalSetup: "./tests-e2e/support/global-setup.ts",
  // Every test boots a daemon of its own, so no file needs its tests in order.
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  // A spec that passes only on a second try is a failure to fix, not hide:
  // CI retries once so a flake is told from a break (the retry's outcome and
  // trace), and `failOnFlakyTests` fails the run either way.
  retries: process.env.CI ? 1 : 0,
  failOnFlakyTests: !!process.env.CI,
  // Locally the default (half the cores) is fastest: on 14 cores the suite
  // took 82 s at 4 workers, 69 s at 7, 70 s at 10.
  workers: process.env.CI ? 3 : undefined,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  // The JUnit report is what oxplow's own collector reads.
  // With `includeRetries`, a test that passed on its retry carries a
  // `flakyFailure` (or `flakyError`): `bun run e2e:flakes` counts those.
  reporter: [["list"], ["junit", { outputFile: "tests-e2e/.output/junit.xml", includeRetries: true }]],
  use: {
    baseURL: PREVIEW,
    // Tracing every test costs ~15% of the suite's CPU (measured locally:
    // 395 s user with, 340 s without, same wall), which a 4-core runner feels.
    // CI traces the retry of a failed test only; a local run, failures.
    trace: process.env.CI ? "on-first-retry" : "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    // A custom component's frame is the one place the two engines are
    // checked apart (macOS's window is WebKit): its sandbox and frame bound.
    { name: "webkit", use: { ...devices["Desktop Safari"] }, testMatch: /specs\/components\// },
  ],
  webServer: {
    // Built once per run, so a spec never meets a stale `dist/`. Locally, a
    // server already on the port is used as it is: `bun run e2e:serve` keeps
    // one up that rebuilds `dist/` on every change (`scripts/e2e-serve.sh`).
    command:
      `bun run --cwd apps/desktop build && bun run --cwd apps/desktop preview -- --host 127.0.0.1 --port ${PORT} --strictPort`,
    url: PREVIEW,
    timeout: 300_000,
    reuseExistingServer: !process.env.CI,
  },
});
