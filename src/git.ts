import { execFileSync } from "node:child_process";

export function detectCurrentBranch(projectDir: string): string | null {
  try {
    const out = execFileSync("git", ["-C", projectDir, "rev-parse", "--abbrev-ref", "HEAD"], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
    if (!out || out === "HEAD") return null;
    return out;
  } catch {
    return null;
  }
}

export function isGitRepo(projectDir: string): boolean {
  try {
    execFileSync("git", ["-C", projectDir, "rev-parse", "--git-dir"], {
      stdio: "ignore",
    });
    return true;
  } catch {
    return false;
  }
}
