// Verify the "Commit (N)" button in the Files view works end-to-end:
// create a scratch dirty file, launch oxplow, click the button, fill message,
// submit, verify commit landed in git log, then safely roll back the test
// commit (only if we actually advanced HEAD).
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { mkdirSync, writeFileSync, unlinkSync, existsSync } from "node:fs";
import { execSync } from "node:child_process";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const outDir = resolve(__dirname, "screenshots");
  mkdirSync(outDir, { recursive: true });

  const scratchRel = "tests-e2e/scratch-commit-target.txt";
  const scratchAbs = resolve(projectDir, scratchRel);
  const startSha = execSync("git rev-parse HEAD", { cwd: projectDir }).toString().trim();

  writeFileSync(scratchAbs, `line one\nline two\n`);
  console.log("[probe] created scratch dirty file; start sha:", startSha);

  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);

    // Activate Files tab.
    const proj = window.getByTestId("dock-panel-project");
    for (let i = 0; i < 3; i++) {
      if ((await proj.getAttribute("data-active")) === "true" && (await proj.isVisible())) break;
      await window.getByTestId("dock-tab-project").click();
      await window.waitForTimeout(300);
    }

    const commitBtn = window.getByTestId("files-commit");
    await commitBtn.waitFor({ state: "visible", timeout: 5_000 });
    const label = await commitBtn.textContent();
    console.log("[probe] commit button text:", label);
    await window.screenshot({ path: resolve(outDir, "fc-01-button.png") });
    await commitBtn.click();
    await window.waitForTimeout(400);

    const msg = window.getByTestId("files-commit-message");
    await msg.fill("chore(test): probe-files-commit scratch commit");
    await window.screenshot({ path: resolve(outDir, "fc-02-dialog.png") });
    await window.getByTestId("files-commit-submit").click();

    await window.waitForTimeout(1_500);
    await window.screenshot({ path: resolve(outDir, "fc-03-after.png") });

    const endSha = execSync("git rev-parse HEAD", { cwd: projectDir }).toString().trim();
    console.log("[probe] end sha:", endSha);
    if (endSha === startSha) throw new Error("no commit landed");
    const subject = execSync("git log -1 --format=%s", { cwd: projectDir }).toString().trim();
    console.log("[probe] HEAD subject:", subject);
    if (!subject.includes("probe-files-commit")) throw new Error(`unexpected subject: ${subject}`);
    console.log("[probe] SUCCESS: commit landed via Files view");
  } finally {
    await close();
    // Only roll back commits WE created on top of startSha. Unconditional
    // `git reset --hard` would destroy any uncommitted outer work — learned
    // the hard way.
    try {
      const currentSha = execSync("git rev-parse HEAD", { cwd: projectDir }).toString().trim();
      if (currentSha !== startSha) {
        execSync(`git reset --hard ${startSha}`, { cwd: projectDir });
        console.log("[probe] rolled back commits on top of", startSha);
      } else {
        console.log("[probe] no commits to roll back; leaving tree alone");
      }
    } catch (e) {
      console.error("[probe] rollback check failed:", e);
    }
    if (existsSync(scratchAbs)) unlinkSync(scratchAbs);
  }
}

runProbe("probe-files-commit", main).catch(() => process.exit(1));
