import { expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// A URI's owner spells it and everyone else carries it. In the desktop the
// one place that writes `oxplow:` or reads a URI is `uri/uri.ts` (builders
// and the parser); every other file takes a URI from a builder or from data
// and passes it on (`.context/resources.md`).

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");

/** Where the scheme may be written: the builders. */
const EXEMPT = [/^uri\//];

/** Hand-built or hand-parsed URIs still to be moved onto the builders.
 *  Empty: add nothing here, use a builder. */
const KEPT: string[] = [];

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "generated" || name === "node_modules" ? [] : sources(path);
    return /\.tsx?$/.test(name) && !/\.(test|spec)\.tsx?$/.test(name) && !/\.d\.ts$/.test(name) ? [path] : [];
  });
}

/** Code without its comments (a doc comment may name an example URI). */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, "")).replace(/(^|[^:])\/\/.*$/gm, "$1");
}

const FORBIDDEN: Array<[RegExp, string]> = [
  [/[`"']oxplow:[a-z][a-z0-9_]*\//, "writes a URI (use a builder in uri/uri.ts)"],
  [/\^oxplow:/, "matches the oxplow: scheme (use isOxplowScheme / parseUri)"],
  [/\b\w*[uU]ri\w*\.indexOf\(["']\?["']\)/, "splits a URI at its query (use parseUri)"],
];

test("no desktop code spells or splits an oxplow URI outside uri/uri.ts", () => {
  const offenders: string[] = [];
  for (const file of sources(ROOT)) {
    const rel = relative(ROOT, file);
    if (EXEMPT.some((re) => re.test(rel))) continue;
    code(readFileSync(file, "utf8"))
      .split("\n")
      .forEach((line, i) => {
        for (const [re, why] of FORBIDDEN) {
          const at = `${rel}:${i + 1}`;
          if (re.test(line) && !KEPT.includes(at)) offenders.push(`${at} ${why}: ${line.trim()}`);
        }
      });
  }
  expect(offenders).toEqual([]);
});
