// Continuation of dogfood-ctx-menu.ts: reopen oxplow, find the
// pending commit point the inner agent proposed, and approve it
// through the UI. This is the "click Approve through oxplow's UI"
// step of the canonical dogfood pass.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchOxplow, probeLog, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    probeLog("[approve] oxplow launched");

    // Work panel should be first dock tab.
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }
    probeLog("[approve] Work panel active");

    await window.screenshot({ path: resolve(outDir, "approve-01-work-panel.png") });

    // Survey: dump every element mentioning commit / approve / propose.
    const survey = await window.evaluate(() => {
      const hits: Array<{ tag: string; testid: string | null; text: string }> = [];
      for (const el of Array.from(document.querySelectorAll("*"))) {
        const t = (el as HTMLElement).innerText ?? "";
        if (t.length > 250 || !t.trim()) continue;
        if (!/commit|approve|propose|pending/i.test(t)) continue;
        const parent = el.parentElement;
        if (parent && (parent.innerText ?? "") === t) continue;
        hits.push({
          tag: el.tagName,
          testid: (el as HTMLElement).dataset?.testid ?? null,
          text: t.trim().slice(0, 200),
        });
        if (hits.length >= 30) break;
      }
      return hits;
    });
    writeFileSync(resolve(outDir, "approve-survey.json"), JSON.stringify(survey, null, 2));
    probeLog(`[approve] DOM survey (${survey.length} hits):`);
    for (const h of survey.slice(0, 15)) {
      probeLog(`[approve]   ${h.tag} testid=${h.testid ?? "-"} | ${h.text.slice(0, 120)}`);
    }

    // Find the first button that looks like an approval action.
    const approvalClick = await window.evaluate(() => {
      const buttons = Array.from(document.querySelectorAll("button"));
      const approve = buttons.find((b) => /approve/i.test(b.textContent ?? "") || /approve/i.test(b.getAttribute("title") ?? ""));
      if (!approve) return { found: false };
      (approve as HTMLElement).click();
      return { found: true, testid: (approve as HTMLElement).dataset?.testid ?? null, text: (approve.textContent ?? "").trim().slice(0, 60) };
    });
    probeLog(`[approve] approve button: ${JSON.stringify(approvalClick)}`);

    await window.waitForTimeout(1_500);
    await window.screenshot({ path: resolve(outDir, "approve-02-after-click.png") });
    probeLog("[approve] done");
  } finally {
    await close();
  }
}

runProbe("dogfood-approve", main, { wallMs: 60_000, silenceMs: 45_000 }).catch(() => process.exit(1));
