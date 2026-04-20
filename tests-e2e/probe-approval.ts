// Relaunch newde post-agent-run and find the commit-approval UI.
// The agent called propose_commit; there should be a pending commit point
// with a message waiting for approval.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[approval]", ...args);

  try {
    await window.waitForTimeout(3_000);
    await window.screenshot({ path: resolve(outDir, "ap-00-launch.png") });

    // Make sure Work panel is active.
    const workPanel = window.getByTestId("dock-panel-plan");
    for (let i = 0; i < 3; i++) {
      if ((await workPanel.getAttribute("data-active")) === "true" && (await workPanel.isVisible())) break;
      await window.getByTestId("dock-tab-plan").click();
      await window.waitForTimeout(300);
    }
    await window.screenshot({ path: resolve(outDir, "ap-01-work-panel.png") });

    // Dump everything that mentions commit / approve / propose / message.
    const sniff = await window.evaluate(() => {
      const out: Array<{ tag: string; testid: string | null; aria: string | null; text: string }> = [];
      for (const el of Array.from(document.querySelectorAll("*"))) {
        const t = (el as HTMLElement).innerText ?? "";
        if (t.length > 250) continue;
        if (!/commit|approve|propose|pending|message/i.test(t)) continue;
        // Skip outer containers that also contain this text.
        const parent = el.parentElement;
        if (parent && (parent.innerText ?? "") === t) continue;
        out.push({
          tag: el.tagName,
          testid: (el as HTMLElement).dataset?.testid ?? null,
          aria: el.getAttribute("aria-label"),
          text: t.trim().slice(0, 160),
        });
        if (out.length >= 40) break;
      }
      return out;
    });
    writeFileSync(resolve(outDir, "ap-sniff.json"), JSON.stringify(sniff, null, 2));
    log("commit/approve hints written to ap-sniff.json (count:", sniff.length, ")");
    for (const s of sniff.slice(0, 20)) {
      console.log("  ", s.tag, s.testid ?? "-", "|", JSON.stringify(s.text));
    }

    // Also dump the full Work panel DOM outline for inspection.
    const outline = await window.evaluate(() => {
      const panel = document.querySelector('[data-testid="dock-panel-plan"]');
      function walk(el: Element, depth: number, limit: { n: number }): string {
        if (limit.n <= 0 || depth > 8) return "";
        limit.n -= 1;
        const tag = el.tagName.toLowerCase();
        const testid = (el as HTMLElement).dataset?.testid ? `[testid=${(el as HTMLElement).dataset.testid}]` : "";
        const aria = el.getAttribute("aria-label") ? `[aria=${el.getAttribute("aria-label")}]` : "";
        const own = Array.from(el.childNodes).filter((c) => c.nodeType === 3).map((c) => (c.textContent ?? "").trim()).filter(Boolean).join(" ").slice(0, 80);
        const head = `${"  ".repeat(depth)}${tag}${testid}${aria}${own ? ` "${own}"` : ""}`;
        const kids: string[] = [];
        for (const k of Array.from(el.children)) {
          const s = walk(k, depth + 1, limit);
          if (s) kids.push(s);
        }
        return kids.length ? `${head}\n${kids.join("\n")}` : head;
      }
      return panel ? walk(panel, 0, { n: 400 }) : "(no plan panel)";
    });
    writeFileSync(resolve(outDir, "ap-plan-outline.txt"), outline);
    log("plan outline written ->", resolve(outDir, "ap-plan-outline.txt"));
  } finally {
    await close();
  }
}

runProbe("probe-approval", main).catch(() => process.exit(1));
