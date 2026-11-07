// Build the binaries the suite runs, once, and hand their paths to the
// workers through the environment.
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { join, relative, resolve } from "node:path";

import { LOG_DIR } from "./daemon.js";

/** The executable cargo built for `bin` (`--message-format=json`). */
function built(output: string, bin: string): string {
  for (const line of output.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name?: string; kind?: string[] }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === bin && msg.executable) return msg.executable;
  }
  throw new Error(`cargo built no ${bin}`);
}

/** The spec files under `dir` that name a timer — Playwright's
 *  `waitForTimeout` or a `setTimeout`, called or not (`test.setTimeout`
 *  is a spec's time limit, not a sleep). Specs wait on what
 *  they expect (web-first `expect`, `waitForModels`, `until`), never on
 *  time: a sleep is either too short (flaky) or too long. */
export function sleepsIn(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) out.push(...sleepsIn(path));
    else if (/\.ts$/.test(name) && /\bwaitForTimeout\b|(?<!test\.)\bsetTimeout\b/.test(readFileSync(path, "utf8"))) out.push(path);
  }
  return out;
}

/** The specs whose window is never ready (a refused token), the only ones
 *  that load the app without waiting for it. */
const WINDOW_NEVER_READY = ["shell/connect.spec.ts"];

/** The specs about the launcher itself, the only ones that type into it. */
const LAUNCHER_SPECS = ["work/launcher-search.spec.ts", "work/goto.spec.ts", "shell/agent-session.spec.ts"];

/** The lines under `dir` that open, look or wait their own way, as
 *  `path:line: what to do instead` (paths relative to `dir`). Specs open
 *  and look the same way, so a race fixed once is fixed everywhere:
 *  - `openFromLauncher(` types into the launcher, the slowest and most
 *    focus-sensitive path: `open(page, daemon, uri)` instead, outside the
 *    launcher's own specs;
 *  - `frameLocator(` skips the scroll that gives a frame below the fold its
 *    animation frames: `inFrame`;
 *  - `page.goto(` and `page.reload(` don't wait for the window, and a click
 *    or navigate before it is ready opens nothing: `load` and `reload`;
 *  - `toHaveCount(0)` on the page itself (`expect(page.…)`) counts what the
 *    hidden tabs hold too, the New Session tab's own form and buttons among
 *    them: under `shown(page)`, or a form/frame/host locator made from it
 *    (a menu, which sits outside the tab, is `not.toBeVisible()`). */
export function uiRulesIn(dir: string, base = dir): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      out.push(...uiRulesIn(path, base));
      continue;
    }
    if (!/\.ts$/.test(name)) continue;
    const rel = relative(base, path);
    readFileSync(path, "utf8")
      .split("\n")
      .forEach((line, i) => {
        const at = `${rel}:${i + 1}:`;
        if (/\bopenFromLauncher\(/.test(line) && !LAUNCHER_SPECS.includes(rel)) out.push(`${at} open the page with open(page, daemon, uri), not the launcher`);
        if (/\bframeLocator\(/.test(line)) out.push(`${at} use inFrame(...), which scrolls the frame into view`);
        if (/\bpage\.(goto|reload)\(/.test(line) && !WINDOW_NEVER_READY.includes(rel)) out.push(`${at} use load(page) or reload(page), which wait until the window is ready`);
        if (/\btoHaveCount\(0\)/.test(line)) {
          // The statement: from its `expect(` (a few lines up, when the call wraps) to here.
          const lines = readFileSync(path, "utf8").split("\n");
          let start = i;
          while (start > i - 4 && start > 0 && !/\bexpect\(/.test(lines[start] ?? "")) start -= 1;
          const statement = lines.slice(start, i + 1).join(" ");
          if (/\bexpect\(\s*(?:page|win)\b/.test(statement) && !/\bshown\(/.test(statement)) {
            out.push(`${at} toHaveCount(0) on the page counts hidden tabs: scope it under shown(page) or a form/frame locator`);
          }
        }
      });
  }
  return out;
}

export default function globalSetup(): void {
  const specs = join(import.meta.dirname, "..", "specs");
  const sleeps = sleepsIn(specs);
  if (sleeps.length > 0) throw new Error(`these specs sleep; wait on what they expect instead:\n${sleeps.join("\n")}`);
  const loose = uiRulesIn(specs).filter((l) => !l.startsWith("harness/"));
  if (loose.length > 0) throw new Error(`these specs open or look their own way:\n${loose.join("\n")}`);
  rmSync(LOG_DIR, { recursive: true, force: true });
  // `OXPLOW_E2E_BIN=target/debug` runs the binaries already there, with no
  // cargo build: for a run of specs only, after a build of your own. The
  // daemon is whatever was built last, so not after a Rust change.
  const prebuilt = process.env.OXPLOW_E2E_BIN && resolve(process.env.OXPLOW_E2E_BIN);
  if (prebuilt) {
    const bins = ["oxplow-daemon-sim", "oxplow-harness-fake", "oxplow-provider-fake", "oxplow-script-worker"];
    const missing = bins.filter((b) => !existsSync(join(prebuilt, b)));
    if (missing.length > 0) throw new Error(`OXPLOW_E2E_BIN=${prebuilt} has no ${missing.join(", ")}`);
    process.env.OXPLOW_E2E_DAEMON = join(prebuilt, "oxplow-daemon-sim");
    process.env.OXPLOW_E2E_HARNESS_FAKE = join(prebuilt, "oxplow-harness-fake");
    process.env.OXPLOW_E2E_PROVIDER_FAKE = join(prebuilt, "oxplow-provider-fake");
    return;
  }
  const output = execFileSync(
    "cargo",
    ["build", "-p", "oxplow-daemon-sim", "-p", "oxplow-harness-fake", "-p", "oxplow-provider-fake", "--message-format=json"],
    { encoding: "utf8", maxBuffer: 256 * 1024 * 1024, stdio: ["ignore", "pipe", "inherit"] },
  );
  // The daemon runs a page's or command's script in this worker, found next
  // to its own binary; without it every spec that runs one fails.
  execFileSync("cargo", ["build", "-p", "oxplow-script", "--bin", "oxplow-script-worker"], { stdio: ["ignore", "inherit", "inherit"] });
  process.env.OXPLOW_E2E_DAEMON = built(output, "oxplow-daemon-sim");
  process.env.OXPLOW_E2E_HARNESS_FAKE = built(output, "oxplow-harness-fake");
  process.env.OXPLOW_E2E_PROVIDER_FAKE = built(output, "oxplow-provider-fake");
}
