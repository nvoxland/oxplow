import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync } from "node:fs";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(2_000);

    // Click the "Files" tab first.
    await window.getByRole("button", { name: "Files", exact: true }).first().click();
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "04-files-tab.png") });

    // Expand src/
    const srcFolder = window.getByRole("button", { name: "📁src", exact: true }).first();
    if (await srcFolder.isVisible().catch(() => false)) {
      await srcFolder.click();
      await window.waitForTimeout(400);
      await window.screenshot({ path: resolve(outDir, "05-src-expanded.png") });
    } else {
      console.log("[probe] src/ folder button not visible");
    }

    // Look for package.json to click and open.
    const pkg = window.getByRole("button", { name: /package\.json/ }).first();
    if (await pkg.isVisible().catch(() => false)) {
      await pkg.click();
      await window.waitForTimeout(1_200);
      await window.screenshot({ path: resolve(outDir, "06-file-opened.png") });
    }

    // Is Monaco rendered?
    const monacoCount = await window.locator(".monaco-editor").count();
    console.log("[probe] .monaco-editor count:", monacoCount);

    // Try Cmd+K (command palette).
    await window.keyboard.press("Meta+K");
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "07-cmd-k.png") });
    const paletteInput = await window.locator('input[placeholder*="ommand"], input[placeholder*="Type a command"], input[placeholder*="earch"]').count();
    console.log("[probe] palette-like input count after Cmd+K:", paletteInput);
    // Escape to close
    await window.keyboard.press("Escape");
    await window.waitForTimeout(300);

    // Try Cmd+P (quick open).
    await window.keyboard.press("Meta+P");
    await window.waitForTimeout(500);
    await window.screenshot({ path: resolve(outDir, "08-cmd-p.png") });
    await window.keyboard.press("Escape");
  } finally {
    await close();
  }
}

runProbe("probe-editor", main).catch(() => process.exit(1));
