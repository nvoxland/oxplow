// Does the React UI actually boot in a plain browser?
//
// `.context/remote-daemon.md` says it should — CORS is permissive "so the
// frontend can run in a plain browser (Playwright-driven UX testing)", shell-
// local channels go inert without a Tauri host, and `no-tauri-imports.test.ts`
// guards the facade. But designed-to and verified are different things, and
// this path has never actually been exercised. This is the smallest script that
// settles it, and the gate for everything else in here.
//
//   node tests-e2e/boot-check.mjs            # expects APP_URL to be serving
//
// See README.md for bringing up the daemon + vite it talks to.

import { chromium } from "playwright";

const APP_URL = process.env.APP_URL ?? "http://localhost:5199/";
const SHOT = process.env.SHOT ?? "tests-e2e/boot.png";

const browser = await chromium.launch();
const page = await browser.newPage();

const pageErrors = [];
const consoleLines = [];
page.on("pageerror", (e) => pageErrors.push(String(e).slice(0, 400)));
page.on("console", (m) => consoleLines.push(`${m.type()}: ${m.text().slice(0, 240)}`));
// A failed /ipc call is the most likely way the browser path breaks, so surface
// them explicitly rather than leaving them buried in console noise.
const failedRequests = [];
page.on("requestfailed", (r) => failedRequests.push(`${r.method()} ${r.url()} — ${r.failure()?.errorText}`));

await page.goto(APP_URL, { waitUntil: "networkidle", timeout: 45_000 });
await page.waitForTimeout(4_000);

const bodyText = (await page.textContent("body")) ?? "";
console.log("title           :", await page.title());
console.log("body chars      :", bodyText.trim().length);
console.log("page errors     :", pageErrors.length);
pageErrors.slice(0, 8).forEach((e) => console.log("   ERR", e));
console.log("failed requests :", failedRequests.length);
failedRequests.slice(0, 8).forEach((r) => console.log("   REQ", r));
console.log("console (last 15):");
consoleLines.slice(-15).forEach((l) => console.log("   ", l));

// The archived Electron probes are a behaviour corpus but their selectors are
// stale, so dump what this build actually exposes — that's the starting point
// for writing any new probe.
const testids = await page
  .locator("[data-testid]")
  .evaluateAll((els) => [...new Set(els.map((e) => e.getAttribute("data-testid")))]);
console.log(`data-testids (${testids.length}):`);
console.log("   ", testids.slice(0, 60).join(", "));

await page.screenshot({ path: SHOT });
console.log("screenshot      :", SHOT);
await browser.close();
