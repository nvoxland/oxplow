// Dogfood probe: open a file from the tree, edit it in Monaco, Cmd+S,
// verify dirty indicator flips and tab label loses the ● prefix. Then
// try to close the tab while dirty and observe the warning behavior.
//
// This probe targets scenario 9 in ux-test.md (open/edit/save/close)
// and B2 in the discovered-during-dogfood backlog.
//
// Runs against the REPO ITSELF as the project dir (same pattern as
// probe-editor.ts and dogfood-cycle-2.ts). The probe edits
// `tests-e2e/scratch-edit-target.txt` — a dedicated throwaway file
// that already exists for this purpose — and reverts the edit at the
// end, never committing the change.

import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

const TARGET_RELATIVE = "tests-e2e/scratch-edit-target.txt";

async function main() {
  const projectDir = resolve(__dirname, "..");
  const targetAbsolute = resolve(projectDir, TARGET_RELATIVE);
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const originalContent = readFileSync(targetAbsolute, "utf8");
  const findings: string[] = [];
  const record = (line: string) => { findings.push(line); console.log("[probe-file-edit]", line); };

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_500);

    // Left dock's default active tab is "plan" (Work), so click "project"
    // (Files) to reveal the file tree.
    await window.getByTestId("dock-tab-project").click();
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "pfe-01-startup.png") });

    // Expand tests-e2e/ directory.
    const testsE2eDir = window.getByTestId("file-tree-entry-tests-e2e").first();
    await testsE2eDir.waitFor({ state: "attached", timeout: 10_000 });
    await testsE2eDir.scrollIntoViewIfNeeded().catch(() => {});
    await testsE2eDir.click();
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "pfe-02-tests-e2e-expanded.png") });

    // Click the target file.
    const targetRow = window.getByTestId(`file-tree-entry-${TARGET_RELATIVE}`).first();
    try {
      await targetRow.waitFor({ state: "attached", timeout: 5_000 });
      await targetRow.scrollIntoViewIfNeeded().catch(() => {});
      await targetRow.click();
      await window.waitForTimeout(1_500);
    } catch (e) {
      record(`FRICTION: target file ${TARGET_RELATIVE} never found: ${e}`);
    }
    await window.screenshot({ path: resolve(outDir, "pfe-03-file-opened.png") });

    // 4. Confirm Monaco is hosting that file.
    const monacoFilePath = await window.evaluate(() => {
      const host = document.querySelector('[data-testid="monaco-host"]');
      return host ? (host as HTMLElement).getAttribute("data-file-path") : null;
    });
    if (monacoFilePath !== TARGET_RELATIVE) {
      record(`FRICTION: monaco-host data-file-path=${JSON.stringify(monacoFilePath)} not ${TARGET_RELATIVE}`);
    } else {
      record(`OK: monaco hosting ${TARGET_RELATIVE}`);
    }

    // 5. Inspect tab label in CenterTabs before editing.
    const tabsTextBefore = await window.evaluate(() => {
      const tabBar = document.querySelector('[data-testid="monaco-host"]')?.closest('[style*="flex-direction: column"]')?.parentElement;
      // Fallback: scan all small labeled pill divs in top area.
      return Array.from(document.querySelectorAll("div")).map(d => (d as HTMLElement).innerText).filter(t => t && t.length < 80 && t.includes("scratch-edit")).slice(0, 5);
    });
    record(`tab labels mentioning scratch-edit before edit: ${JSON.stringify(tabsTextBefore)}`);

    // 6. Focus the editor and type.
    const host = window.getByTestId("monaco-host");
    await host.click();
    await window.waitForTimeout(300);
    await window.keyboard.press("End");
    await window.keyboard.type(" // probe edit");
    await window.waitForTimeout(600);
    await window.screenshot({ path: resolve(outDir, "pfe-04-after-edit.png") });

    // 7. Re-inspect tab label — expect "● scratch-edit-target.txt".
    const tabsTextAfter = await window.evaluate(() =>
      Array.from(document.querySelectorAll("div"))
        .map(d => (d as HTMLElement).innerText)
        .filter(t => t && t.length < 80 && t.includes("scratch-edit"))
        .slice(0, 5)
    );
    record(`tab labels mentioning scratch-edit after edit: ${JSON.stringify(tabsTextAfter)}`);
    const hasDirtyDot = tabsTextAfter.some((t) => t.includes("●"));
    if (!hasDirtyDot) {
      record("FRICTION: no ● dirty marker on tab label after editing");
    } else {
      record("OK: ● dirty marker present on tab label");
    }

    // 8. Cmd+S to save.
    await window.keyboard.press("Meta+S");
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "pfe-05-after-save.png") });

    const tabsTextSaved = await window.evaluate(() =>
      Array.from(document.querySelectorAll("div"))
        .map(d => (d as HTMLElement).innerText)
        .filter(t => t && t.length < 80 && t.includes("scratch-edit"))
        .slice(0, 5)
    );
    record(`tab labels mentioning scratch-edit after save: ${JSON.stringify(tabsTextSaved)}`);
    const stillDirty = tabsTextSaved.some((t) => t.includes("●"));
    if (stillDirty) {
      record("FRICTION: ● dirty marker persists after Cmd+S");
    } else {
      record("OK: dirty marker cleared after save");
    }

    const diskAfterSave = readFileSync(targetAbsolute, "utf8");
    if (!diskAfterSave.includes("// probe edit")) {
      record("FRICTION: Cmd+S did not actually write to disk");
    } else {
      record("OK: Cmd+S wrote edit to disk");
    }

    // 9. Make another edit, then try closing the tab — look for prompt.
    await host.click();
    await window.waitForTimeout(200);
    await window.keyboard.press("End");
    await window.keyboard.type(" // second edit");
    await window.waitForTimeout(500);

    // Close should now surface a confirm dialog. Arm a dismiss-by-cancel
    // handler first; we expect the first attempt to be cancelled so the
    // tab stays open and the edits stay alive.
    let dialogSeen: string | null = null;
    window.once("dialog", async (d) => { dialogSeen = d.message(); await d.dismiss(); });

    // Locate the tab close X (beside the label). We click the × inside the tab.
    const closeResult = await window.evaluate(() => {
      const candidates = Array.from(document.querySelectorAll("button"));
      for (const b of candidates) {
        if (b.getAttribute("title")?.startsWith("Close ") && b.getAttribute("title")?.includes("scratch-edit")) {
          b.click();
          return { clicked: true, title: b.getAttribute("title") };
        }
      }
      return { clicked: false };
    });
    record(`close button click result: ${JSON.stringify(closeResult)}`);
    await window.waitForTimeout(800);
    await window.screenshot({ path: resolve(outDir, "pfe-06-close-attempt.png") });

    record(`confirm dialog message: ${JSON.stringify(dialogSeen)}`);
    if (!dialogSeen) {
      record("FRICTION: closing a dirty tab did not warn — silent loss of unsaved edits");
    } else {
      record("OK: dirty-close raised a confirm dialog");
    }

    // Since we dismissed the confirm, the tab must still be open.
    const stillOpen = await window.evaluate(() =>
      !!document.querySelector('[data-testid="monaco-host"][data-file-path$="scratch-edit-target.txt"]')
    );
    record(`tab still open after dismiss: ${stillOpen}`);
    if (dialogSeen && !stillOpen) {
      record("FRICTION: confirm was dismissed but tab closed anyway");
    }

    // 10. Verify the second (unsaved) edit never reached disk — it was
    // dismissed with the tab still open, so disk should still hold just
    // the first saved edit.
    const diskFinal = readFileSync(targetAbsolute, "utf8");
    if (diskFinal.includes("// second edit")) {
      record("FRICTION: unsaved second edit reached disk without Cmd+S");
    } else {
      record("OK: unsaved second edit stayed in memory");
    }

    await window.screenshot({ path: resolve(outDir, "pfe-07-final.png") });
  } finally {
    // Always revert the target file.
    writeFileSync(targetAbsolute, originalContent, "utf8");
    await close();
    console.log("\n=== FINDINGS ===");
    for (const f of findings) console.log("  " + f);
    const frictions = findings.filter(f => f.startsWith("FRICTION"));
    console.log(`\nFRICTION count: ${frictions.length}`);
  }
}

runProbe("probe-file-edit", main).catch(() => process.exit(1));
