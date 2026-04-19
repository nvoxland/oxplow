import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchNewde(projectDir);
  try {
    await window.waitForTimeout(4_000);
    // Make sure Work tab is active.
    await window.getByRole("button", { name: "Work", exact: true }).first().click().catch(() => {});
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "02-before.png") });

    // Count all buttons by type BEFORE clicking anything — captures current state.
    const pre = await window.evaluate(() => {
      const bs = Array.from(document.querySelectorAll("button")) as HTMLButtonElement[];
      const counts: Record<string, number> = {};
      for (const b of bs) counts[b.type] = (counts[b.type] ?? 0) + 1;
      return { total: bs.length, counts };
    });
    console.log("[probe] pre-click button counts:", JSON.stringify(pre));

    // Try to find and click "+ New work item" by text.
    const btn = window.getByText("+ New work item", { exact: true }).first();
    console.log("[probe] visible?", await btn.isVisible().catch(() => false));
    console.log("[probe] count:", await btn.count());
    await btn.scrollIntoViewIfNeeded().catch(() => {});
    await btn.click({ timeout: 10_000 });
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "03-after-click.png") });

    // Dump inputs on page to see what the form exposes.
    const inputs = await window.evaluate(() => {
      return Array.from(document.querySelectorAll("input, textarea, select, button")).map((el) => {
        const e = el as HTMLElement;
        return {
          tag: el.tagName,
          type: (el as HTMLInputElement).type || null,
          name: (el as HTMLInputElement).name || null,
          placeholder: (el as HTMLInputElement).placeholder || null,
          ariaLabel: e.getAttribute("aria-label"),
          text: e.textContent?.trim().slice(0, 60) || null,
          testid: e.dataset?.testid || null,
        };
      });
    });
    writeFileSync(resolve(outDir, "03-form-inputs.json"), JSON.stringify(inputs, null, 2));
    console.log("[probe] form inputs dumped");
    console.log(inputs.slice(0, 30));
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[probe] failed:", err);
  process.exit(1);
});
