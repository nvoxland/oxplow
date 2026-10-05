// The browser suite (P11, `tests-e2e/README.md`): the built frontend, served
// by `vite preview`, against a real daemon — `oxplow-daemon-sim`, secrets in
// memory — over a throwaway git project, one per worker.
import { defineConfig, devices } from "@playwright/test";

const PREVIEW = "http://127.0.0.1:4173";

export default defineConfig({
  testDir: "tests-e2e/specs",
  outputDir: "tests-e2e/.output/results",
  globalSetup: "./tests-e2e/support/global-setup.ts",
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  // A spec that passes only on a second try is a failure to fix, not hide.
  retries: 0,
  workers: process.env.CI ? 2 : undefined,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  // The JUnit report is what oxplow's own collector reads.
  reporter: [["list"], ["junit", { outputFile: "tests-e2e/.output/junit.xml" }]],
  use: {
    baseURL: PREVIEW,
    trace: "retain-on-failure",
  },
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    // A custom component's frame is the one place the two engines are
    // checked apart (macOS's window is WebKit): its sandbox and frame bound.
    { name: "webkit", use: { ...devices["Desktop Safari"] }, testMatch: /specs\/components\// },
  ],
  webServer: {
    // Built once per run, so a spec never meets a stale `dist/`.
    command:
      "bun run --cwd apps/desktop build && bun run --cwd apps/desktop preview -- --host 127.0.0.1 --port 4173 --strictPort",
    url: PREVIEW,
    timeout: 300_000,
    reuseExistingServer: false,
  },
});
