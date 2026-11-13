import { expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// The desktop reads the task interface — `v_oxplow_task` and its views,
// whichever list is active — and never oxplow's own task tables, ids or
// refs (`.context/tasks.md`). oxtask is one implementation of the
// interface; nothing here may know which is active. (`v_oxplow_task` is the
// interface itself, so reading it is fine.)

const ROOT = import.meta.dir;

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "generated" ? [] : sources(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}

/** Code without its comments (a doc comment may name an example ref). */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1");
}

const FORBIDDEN: Array<[RegExp, string]> = [
  [/tsk\\d|tsk\[0-9\]|["'`]tsk["'`$]/, "parses oxplow's task ids (ids follow the active list's id pattern)"],
];

test("no desktop code reads oxplow's task implementation", () => {
  const offenders: string[] = [];
  for (const file of sources(ROOT)) {
    const lines = code(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, i) => {
      for (const [re, why] of FORBIDDEN) {
        if (re.test(line)) offenders.push(`${relative(ROOT, file)}:${i + 1} ${why}: ${line.trim()}`);
      }
    });
  }
  expect(offenders).toEqual([]);
});
