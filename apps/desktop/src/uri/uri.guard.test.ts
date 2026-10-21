import { expect, test } from "bun:test";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// A ref's owner spells it and everyone else carries it. In the desktop the
// one place that writes a kind prefix is `uri/uri.ts` (builders) and the
// one place that reads one is `parseRef`; every other file takes a ref from
// a builder or from data and passes it on (`.context/resources.md`, "The TS side
// is on canonical refs"). The kinds are the shared fixture's, the same list
// the Rust registry is asserted against.

const HERE = dirname(fileURLToPath(import.meta.url));
const ROOT = join(HERE, "..");
const FIXTURE = join(HERE, "../../../../crates/oxplow-domain/tests/fixtures/ref_grammar.json");
const KINDS: string[] = JSON.parse(readFileSync(FIXTURE, "utf8")).kinds;

/** Where a prefix may be written: the builders, the revision reader (a
 *  revision's `git:` / `snap:` kinds are not refs) and the page-kind
 *  table. */
const EXEMPT = [/^uri\//, /^revision\.ts$/, /^pageKinds\.tsx$/];

/** Hand-built or hand-parsed refs still to be moved onto the builders.
 *  Empty: add nothing here, use a builder. */
const KEPT: string[] = [];

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === "generated" || name === "node_modules" ? [] : sources(path);
    return /\.tsx?$/.test(name) && !/\.(test|spec)\.tsx?$/.test(name) && !/\.d\.ts$/.test(name) ? [path] : [];
  });
}

/** Code without its comments (a doc comment may name an example ref). */
function code(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, (m) => m.replace(/[^\n]/g, "")).replace(/(^|[^:])\/\/.*$/gm, "$1");
}

const kinds = KINDS.join("|");
const FORBIDDEN: Array<[RegExp, string]> = [
  [new RegExp("`(?:" + kinds + "):\\$\\{"), "builds a ref in a template (use the builder in uri/uri.ts)"],
  [new RegExp("[\"'](?:" + kinds + "):[\"']\\s*\\+"), "builds a ref by concatenation (use the builder in uri/uri.ts)"],
  [new RegExp("startsWith\\([\"'`](?:" + kinds + "):[\"'`]"), "parses a ref by prefix (use parseRef / idOf)"],
  [new RegExp("slice\\([\"'`](?:" + kinds + "):[\"'`]\\.length"), "parses a ref by slicing its prefix (use parseRef / idOf)"],
  [new RegExp("replace\\(/\\^(?:" + kinds + "):"), "parses a ref by replacing its prefix (use parseRef / idOf)"],
  [/\b\w*[rR]ef\w*\.indexOf\(["']:["']\)/, "splits a ref at its colon (use parseRef / kindOf)"],
];

test("no desktop code spells or splits a ref outside uri/uri.ts", () => {
  expect(KINDS.length).toBeGreaterThan(20);
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
