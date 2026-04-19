// Actual end-to-end dogfood: open a scratch file via newde's file tree, type
// into Monaco, save with Cmd+S, verify the bytes on disk. This is the harness
// proving it can *drive* newde to make a real edit.
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { launchNewde } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  // Scratch file that we will edit through newde's UI.
  const scratchRel = "tests-e2e/scratch-edit-target.txt";
  const scratchAbs = resolve(projectDir, scratchRel);
  const originalContent = `original line\n`;
  writeFileSync(scratchAbs, originalContent);

  const { window, close } = await launchNewde(projectDir);
  const log = (...args: unknown[]) => console.log("[drive]", ...args);

  try {
    await window.waitForTimeout(3_000);

    // Step 1: activate Files tab via testid. Click until the panel becomes
    // active — the rail toggles open/closed on repeat clicks, so one click
    // may close the dock if Files was already the active tab and dock was
    // open, etc. Cap at 3 clicks.
    log("step 1: click Files tab");
    const projectPanel = window.getByTestId("dock-panel-project");
    for (let i = 0; i < 3; i++) {
      const active = await projectPanel.getAttribute("data-active");
      const visible = await projectPanel.isVisible();
      if (active === "true" && visible) break;
      await window.getByTestId("dock-tab-project").click();
      await window.waitForTimeout(400);
    }
    if (!(await projectPanel.isVisible())) {
      throw new Error("could not activate Files panel");
    }

    // Step 2: expand tests-e2e/ folder
    log("step 2: expand tests-e2e/");
    const folder = window.getByTestId("file-tree-entry-tests-e2e");
    await folder.waitFor({ state: "visible", timeout: 5_000 });
    if ((await folder.getAttribute("data-expanded")) !== "true") {
      await folder.click();
      await window.waitForTimeout(400);
    }

    // Step 3: click the scratch file to open it in Monaco
    log("step 3: open scratch file");
    const fileBtn = window.getByTestId(`file-tree-entry-${scratchRel}`);
    await fileBtn.waitFor({ state: "visible", timeout: 5_000 });
    await fileBtn.click();
    await window.waitForTimeout(1_500);
    await window.screenshot({ path: resolve(outDir, "10-file-opened.png") });

    // Step 4: verify Monaco loaded the content
    const monacoHost = window.getByTestId("monaco-host");
    await monacoHost.waitFor({ state: "visible", timeout: 5_000 });
    const shownPath = await monacoHost.getAttribute("data-file-path");
    log("monaco file-path attr:", shownPath);
    if (shownPath !== scratchRel) {
      throw new Error(`Monaco is showing "${shownPath}", expected "${scratchRel}"`);
    }

    // Step 5: click into the Monaco view area to get a real cursor, then type.
    // Typing directly into the hidden textarea doesn't route through Monaco's
    // input pipeline — it needs focus via a click on the rendered view.
    log("step 5: focus Monaco and type");
    const viewLines = window.locator('[data-testid="monaco-host"] .view-lines');
    await viewLines.waitFor({ state: "visible", timeout: 5_000 });
    await viewLines.click();
    await window.waitForTimeout(200);
    await window.keyboard.press("Meta+End");
    await window.keyboard.type("APPENDED BY PLAYWRIGHT\n");
    await window.waitForTimeout(300);
    // Read what Monaco thinks the content is.
    const monacoValue = await window.evaluate(() => {
      const host = document.querySelector('[data-testid="monaco-host"]') as HTMLElement | null;
      const lines = host?.querySelector(".view-lines");
      return lines ? (lines as HTMLElement).innerText : null;
    });
    log("monaco view-lines text:", JSON.stringify(monacoValue));

    // Step 6: save with Cmd+S
    log("step 6: Cmd+S save");
    await window.keyboard.press("Meta+S");
    await window.waitForTimeout(1_000);
    await window.screenshot({ path: resolve(outDir, "11-after-save.png") });

    // Step 7: verify on disk
    const afterContent = readFileSync(scratchAbs, "utf8");
    log("disk content:", JSON.stringify(afterContent));
    if (!afterContent.includes("APPENDED BY PLAYWRIGHT")) {
      throw new Error(`Save did not write to disk. Content: ${JSON.stringify(afterContent)}`);
    }
    if (!afterContent.startsWith("original line")) {
      throw new Error(`Original content lost. Content: ${JSON.stringify(afterContent)}`);
    }
    log("OK — Monaco edit roundtripped to disk");

    // Step 8: Cmd+K palette check
    log("step 8: Cmd+K palette");
    await window.keyboard.press("Meta+K");
    await window.waitForTimeout(500);
    const paletteInput = window.getByTestId("command-palette-input");
    const paletteVisible = await paletteInput.isVisible().catch(() => false);
    log("palette visible after Cmd+K:", paletteVisible);
    await window.screenshot({ path: resolve(outDir, "12-palette.png") });
    if (!paletteVisible) throw new Error("Cmd+K did not open the palette");
    await window.keyboard.press("Escape");

    log("SUCCESS: full drive-edit cycle completed");
  } finally {
    await close();
    // Restore scratch file so subsequent runs start clean.
    if (existsSync(scratchAbs)) writeFileSync(scratchAbs, originalContent);
  }
}

main().catch((err) => {
  console.error("[drive] failed:", err);
  process.exit(1);
});
