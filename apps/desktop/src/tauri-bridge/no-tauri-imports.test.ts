// Guard: native access funnels through the transport facade.
//
// Importing `@tauri-apps/*` anywhere under apps/desktop/src EXCEPT
// tauri-bridge/ leaks a native assumption past the switchable
// transport facade — exactly the class of seam bug (CORS,
// menu:command/transformCallback) that breaks "works in Tauri, breaks
// in the browser" silently. This test makes that a CI failure at
// authoring time. To add a new native capability, wrap it in a
// tauri-bridge/ module and import that instead.
//
// The repo has no ESLint toolchain (frontend checks are tsc + bun
// test only); this source-scan guard delivers the same invariant in
// the existing `bun test` step with no new dependency.

import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const SRC_DIR = join(import.meta.dir, "..");
const BRIDGE_DIR = join(SRC_DIR, "tauri-bridge");
const IMPORT_RE = /(?:import|export)[^;]*from\s*["']@tauri-apps\/[^"']+["']/;

function walk(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      out.push(...walk(full));
    } else if (/\.(ts|tsx)$/.test(name)) {
      out.push(full);
    }
  }
  return out;
}

describe("tauri-bridge facade", () => {
  test("no @tauri-apps imports outside tauri-bridge/", () => {
    const offenders: string[] = [];
    for (const file of walk(SRC_DIR)) {
      if (file.startsWith(BRIDGE_DIR)) continue;
      const body = readFileSync(file, "utf8");
      if (IMPORT_RE.test(body)) {
        offenders.push(file.slice(SRC_DIR.length + 1));
      }
    }
    expect(
      offenders,
      `These files import @tauri-apps/* directly. Wrap the native access in a ` +
        `tauri-bridge/ module and import that instead:\n  ${offenders.join("\n  ")}`,
    ).toEqual([]);
  });

  test("the guard regex actually matches a tauri import", () => {
    // Pins the detector so a future refactor can't silently neuter it.
    expect(IMPORT_RE.test(`import { open } from "@tauri-apps/plugin-dialog";`)).toBe(true);
    expect(IMPORT_RE.test(`export { x } from "@tauri-apps/api/core";`)).toBe(true);
    expect(IMPORT_RE.test(`import { foo } from "./local";`)).toBe(false);
  });
});
