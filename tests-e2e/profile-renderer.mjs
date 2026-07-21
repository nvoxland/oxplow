// CPU-profile the renderer in headless Chromium (tsk219/tsk223).
//
// Every performance measurement this project had taken was of the Rust backend;
// the renderer had never been profiled at all. This drives the real React UI
// against `oxplow-daemon` over the `VITE_OXPLOW_REMOTE` transport switch and
// captures a V8 CPU profile via CDP.
//
//   APP_URL=http://localhost:5199/ SECONDS=30 node tests-e2e/profile-renderer.mjs
//
// ⚠️ This is CHROMIUM, not the shipped WKWebView. It is a good proxy for
// React/JS work — which is what the idle-timer hypotheses are — and a poor one
// for paint/scroll/GC behaviour. Do not report it as a measurement of the
// shipped renderer.
//
// Ranking note, learned the hard way on the Rust side (.context/performance.md):
// rank by SELF time actually attributed to samples, and treat an idle profile's
// "(program)"/"(idle)" nodes as what they are — not as work.

import { chromium } from "playwright";
import { writeFileSync } from "node:fs";

const APP_URL = process.env.APP_URL ?? "http://localhost:5199/";
const SECONDS = Number(process.env.SECONDS ?? 20);
const OUT = process.env.OUT ?? "tests-e2e/renderer-profile.json";
/** Optional data-testid to click before profiling — used to
 *  expand a collapsed section so its rows are actually mounted (e.g.
 *  rail-section-toggle-work). A profile of a
 *  collapsed list measures nothing, which is easy to do by accident. */
const CLICK_TESTID = process.env.CLICK_TESTID;
const SHOT = process.env.SHOT;

const browser = await chromium.launch();
const page = await browser.newPage();
await page.goto(APP_URL, { waitUntil: "networkidle", timeout: 45_000 });
// Let boot settle so we profile steady state, not first paint.
await page.waitForTimeout(5_000);

if (CLICK_TESTID) {
  await page.getByTestId(CLICK_TESTID).first().click();
  await page.waitForTimeout(2_500);
  const mounted = await page.locator("[data-testid]").count();
  console.log(`clicked [${CLICK_TESTID}] · elements with a testid now: ${mounted}`);
}
if (SHOT) await page.screenshot({ path: SHOT });

const client = await page.context().newCDPSession(page);
await client.send("Profiler.enable");
// 100µs sampling: fine enough to see a 12.5 Hz re-render, cheap enough not to
// distort what it measures.
await client.send("Profiler.setSamplingInterval", { interval: 100 });
await client.send("Profiler.start");

console.log(`profiling ${SECONDS}s of IDLE (no interaction)…`);
await page.waitForTimeout(SECONDS * 1000);

const { profile } = await client.send("Profiler.stop");
writeFileSync(OUT, JSON.stringify(profile));

// --- attribute self time per node -------------------------------------------
const byId = new Map(profile.nodes.map((n) => [n.id, n]));
const selfSamples = new Map();
for (const id of profile.samples ?? []) {
  selfSamples.set(id, (selfSamples.get(id) ?? 0) + 1);
}
const totalMs = (profile.endTime - profile.startTime) / 1000;
const sampleCount = (profile.samples ?? []).length;

const label = (n) => {
  const f = n.callFrame;
  const name = f.functionName || "(anonymous)";
  const url = (f.url || "").replace(/^https?:\/\/[^/]+\//, "").replace(/\?.*$/, "");
  return url ? `${name}  ${url}:${f.lineNumber + 1}` : name;
};

const rows = [...selfSamples.entries()]
  .map(([id, count]) => ({ node: byId.get(id), count }))
  .filter((r) => r.node)
  .sort((a, b) => b.count - a.count);

// Chromium reports doing-nothing as (idle)/(program); separating them is the
// difference between "the renderer is busy" and "the renderer is asleep".
const IDLE = new Set(["(idle)", "(program)", "(garbage collector)", "(root)"]);
const busy = rows.filter((r) => !IDLE.has(r.node.callFrame.functionName));
const idleSamples = rows
  .filter((r) => IDLE.has(r.node.callFrame.functionName))
  .reduce((a, r) => a + r.count, 0);
const busySamples = busy.reduce((a, r) => a + r.count, 0);

console.log(`\nwall: ${totalMs.toFixed(0)} ms · samples: ${sampleCount}`);
console.log(
  `idle/program: ${((100 * idleSamples) / sampleCount).toFixed(1)}%  ·  ` +
    `actually executing JS: ${((100 * busySamples) / sampleCount).toFixed(1)}%`,
);
console.log(`\ntop JS by self time (share of ALL samples):`);
for (const r of busy.slice(0, 25)) {
  const pct = (100 * r.count) / sampleCount;
  console.log(`  ${pct.toFixed(2).padStart(6)}%  ${label(r.node)}`);
}
console.log(`\nwrote ${OUT}`);
await browser.close();
