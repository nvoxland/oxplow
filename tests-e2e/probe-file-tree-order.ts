/**
 * Probe: file-tree root puts source-y directories before known dev-noise
 * dirs (.claude, .oxplow, node_modules, etc.). First-time users shouldn't
 * scroll past .claude to find src/.
 */
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { launchOxplow, runProbe } from "./harness.ts";

const __dirname = dirname(fileURLToPath(import.meta.url));

async function main() {
  const projectDir = resolve(__dirname, "..");
  const { window, close } = await launchOxplow(projectDir);
  try {
    await window.waitForTimeout(3_000);
    await window.getByTestId("dock-tab-project").click();
    await window.waitForTimeout(500);

    // Read the root-level directory names in DOM order.
    const names = await window.evaluate(() => {
      const nodes = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid^="file-tree-entry-"][data-kind="directory"]'),
      ).filter((n) => n.offsetParent !== null);
      // Root dirs have no slash in their path.
      return nodes
        .map((n) => n.dataset.testid!.replace("file-tree-entry-", ""))
        .filter((p) => !p.includes("/"));
    });
    console.log("[probe] root dirs:", names.join(", "));

    const srcIdx = names.indexOf("src");
    const claudeIdx = names.indexOf(".claude");
    const nodeModulesIdx = names.indexOf("node_modules");

    if (srcIdx === -1) {
      console.log("[probe] FAIL: 'src' not present at root");
      process.exit(2);
    }
    if (claudeIdx !== -1 && srcIdx > claudeIdx) {
      console.log("[probe] FAIL: 'src' comes after '.claude'");
      process.exit(3);
    }
    if (nodeModulesIdx !== -1 && srcIdx > nodeModulesIdx) {
      console.log("[probe] FAIL: 'src' comes after 'node_modules'");
      process.exit(4);
    }
    console.log("[probe] OK: source dirs lead; dev-noise dirs pushed to bottom");
  } finally {
    await close();
  }
}

runProbe("probe-file-tree-order", main).catch(() => process.exit(1));
