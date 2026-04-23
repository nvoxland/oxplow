import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync } from "node:fs";
import { launchOxplow } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  console.log(`[smoke] launching oxplow against ${projectDir}`);
  const { app, window, close } = await launchOxplow(projectDir);

  try {
    // Give the app a beat to mount the React tree.
    await window.waitForTimeout(2_000);

    const shot = resolve(outDir, "01-initial.png");
    await window.screenshot({ path: shot, fullPage: false });
    console.log(`[smoke] screenshot -> ${shot}`);

    const title = await window.title();
    console.log(`[smoke] window title: ${title}`);

    // Print a condensed outline of the DOM so I can find stable selectors.
    const outline = await window.evaluate(() => {
      function summarize(el: Element, depth: number, limit: { n: number }): string {
        if (limit.n <= 0 || depth > 6) return "";
        limit.n -= 1;
        const tag = el.tagName.toLowerCase();
        const id = el.id ? `#${el.id}` : "";
        const cls = el.className && typeof el.className === "string" ? `.${el.className.split(/\s+/).slice(0, 2).join(".")}` : "";
        const testid = (el as HTMLElement).dataset?.testid ? `[data-testid=${(el as HTMLElement).dataset.testid}]` : "";
        const text = (el.textContent ?? "").trim().slice(0, 60).replace(/\s+/g, " ");
        const head = `${"  ".repeat(depth)}${tag}${id}${cls}${testid}${text ? ` "${text}"` : ""}`;
        const kids: string[] = [];
        for (const k of Array.from(el.children)) {
          const s = summarize(k, depth + 1, limit);
          if (s) kids.push(s);
        }
        return kids.length ? `${head}\n${kids.join("\n")}` : head;
      }
      return summarize(document.body, 0, { n: 200 });
    });

    const outlinePath = resolve(outDir, "01-outline.txt");
    writeFileSync(outlinePath, outline);
    console.log(`[smoke] outline -> ${outlinePath} (${outline.split("\n").length} lines)`);
    console.log(outline.split("\n").slice(0, 40).join("\n"));
  } finally {
    await close();
  }
}

main().catch((err) => {
  console.error("[smoke] failed:", err);
  process.exit(1);
});
