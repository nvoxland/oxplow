import { describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  CodeQualityToolMissingError,
  parseJscpdReport,
  parseLizardCsv,
  runJscpd,
  runLizard,
} from "./code-quality.js";

function isOnPath(cmd: string): boolean {
  try {
    execFileSync("which", [cmd], { stdio: ["ignore", "ignore", "ignore"] });
    return true;
  } catch {
    return false;
  }
}

const HAS_LIZARD = isOnPath("lizard");
const HAS_JSCPD = isOnPath("jscpd");

describe("parseLizardCsv", () => {
  test("emits three findings per function (complexity, length, parameter-count)", () => {
    const csv = `12,5,80,2,18,doStuff@10-27@/abs/proj/src/foo.ts\n3,1,12,0,5,trivial@30-34@/abs/proj/src/foo.ts\n`;
    const findings = parseLizardCsv(csv, "/abs/proj");
    expect(findings).toHaveLength(6);
    const doStuff = findings.filter((f) => (f.extra as { functionName?: string })?.functionName === "doStuff");
    expect(doStuff.map((f) => f.kind).sort()).toEqual(["complexity", "function-length", "parameter-count"]);
    const complexity = doStuff.find((f) => f.kind === "complexity")!;
    expect(complexity.metricValue).toBe(5);
    expect(complexity.path).toBe("src/foo.ts");
    expect(complexity.startLine).toBe(10);
    expect(complexity.endLine).toBe(27);
  });

  test("ignores blank lines and malformed rows", () => {
    const csv = `\n\nnot,enough,cols\n12,5,80,2,18,doStuff@10-27@/abs/proj/src/foo.ts\n`;
    const findings = parseLizardCsv(csv, "/abs/proj");
    expect(findings).toHaveLength(3);
  });

  test("handles location with @ in the filename", () => {
    const csv = `12,5,80,2,18,doStuff@10-27@/abs/proj/weird@dir/foo.ts\n`;
    const findings = parseLizardCsv(csv, "/abs/proj");
    expect(findings[0]!.path).toBe("weird@dir/foo.ts");
  });
});

describe("parseJscpdReport", () => {
  test("emits one finding per side of each duplicate pair", () => {
    const report = JSON.stringify({
      duplicates: [
        {
          firstFile: { name: "/abs/proj/a.ts", start: 10, end: 25 },
          secondFile: { name: "/abs/proj/b.ts", start: 80, end: 95 },
          lines: 16,
        },
      ],
    });
    const findings = parseJscpdReport(report, "/abs/proj");
    expect(findings).toHaveLength(2);
    expect(findings[0]!.path).toBe("a.ts");
    expect(findings[0]!.kind).toBe("duplicate-block");
    expect(findings[0]!.metricValue).toBe(16);
    expect((findings[0]!.extra as { peerPath: string }).peerPath).toBe("b.ts");
    expect(findings[1]!.path).toBe("b.ts");
    expect((findings[1]!.extra as { peerPath: string }).peerPath).toBe("a.ts");
  });

  test("falls back to start/end if `lines` is missing", () => {
    const report = JSON.stringify({
      duplicates: [
        {
          firstFile: { name: "/abs/proj/a.ts", start: 10, end: 25 },
          secondFile: { name: "/abs/proj/b.ts", start: 80, end: 95 },
        },
      ],
    });
    const findings = parseJscpdReport(report, "/abs/proj");
    expect(findings[0]!.metricValue).toBe(16);
  });

  test("returns empty list on malformed JSON", () => {
    expect(parseJscpdReport("not json", "/abs/proj")).toEqual([]);
  });

  test("returns empty list when no duplicates field", () => {
    expect(parseJscpdReport("{}", "/abs/proj")).toEqual([]);
  });
});

describe.skipIf(!HAS_LIZARD)("runLizard (integration)", () => {
  test("produces findings against a fixture project", async () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-lizard-fixture-"));
    writeFileSync(
      join(dir, "complex.py"),
      `def deeply_nested(x, y, z, w):
    if x:
        if y:
            if z:
                if w:
                    return 1
                else:
                    return 2
            else:
                return 3
        else:
            return 4
    else:
        return 5
`,
    );
    const findings = await runLizard(dir);
    expect(findings.length).toBeGreaterThan(0);
    const complexity = findings.find((f) => f.kind === "complexity");
    expect(complexity).toBeDefined();
    expect(complexity!.metricValue).toBeGreaterThan(1);
  });
});

describe.skipIf(!HAS_JSCPD)("runJscpd (integration)", () => {
  test("produces findings against a fixture project with duplicate code", async () => {
    const dir = mkdtempSync(join(tmpdir(), "oxplow-jscpd-fixture-"));
    const block = Array.from({ length: 30 }, (_, i) => `  console.log("line ${i}");`).join("\n");
    writeFileSync(join(dir, "a.ts"), `export function a() {\n${block}\n}\n`);
    writeFileSync(join(dir, "b.ts"), `export function b() {\n${block}\n}\n`);
    const findings = await runJscpd(dir);
    expect(findings.length).toBeGreaterThan(0);
    expect(findings.every((f) => f.kind === "duplicate-block")).toBe(true);
  });
});

test("runLizard surfaces CodeQualityToolMissingError when binary is not on PATH", async () => {
  // Force ENOENT by giving an empty PATH. Bun preserves env unless overridden.
  const originalPath = process.env.PATH;
  process.env.PATH = "/nonexistent-oxplow-path";
  try {
    await runLizard("/tmp");
    throw new Error("expected throw");
  } catch (err) {
    expect(err).toBeInstanceOf(CodeQualityToolMissingError);
  } finally {
    process.env.PATH = originalPath;
  }
});
