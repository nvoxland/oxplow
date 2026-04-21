import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Ensure `<projectDir>/.newde/` exists and carries a `.gitignore` with `*`
 *  so its contents are invisible to git without the user editing their
 *  project's own .gitignore. Idempotent; safe to call on every startup
 *  path that may be the first to touch `.newde/`. */
export function ensureNewdeRoot(projectDir: string): string {
  const rootDir = join(projectDir, ".newde");
  mkdirSync(rootDir, { recursive: true });
  const gitignorePath = join(rootDir, ".gitignore");
  if (!existsSync(gitignorePath)) writeFileSync(gitignorePath, "*\n");
  return rootDir;
}
