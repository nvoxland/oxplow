import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

/** Ensure `<projectDir>/.oxplow/` exists and carries a `.gitignore` with `*`
 *  so its contents are invisible to git without the user editing their
 *  project's own .gitignore. Idempotent; safe to call on every startup
 *  path that may be the first to touch `.oxplow/`. */
export function ensureOxplowRoot(projectDir: string): string {
  const rootDir = join(projectDir, ".oxplow");
  mkdirSync(rootDir, { recursive: true });
  const gitignorePath = join(rootDir, ".gitignore");
  if (!existsSync(gitignorePath)) writeFileSync(gitignorePath, "*\n");
  return rootDir;
}
