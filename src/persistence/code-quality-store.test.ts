import { beforeEach, describe, expect, test } from "bun:test";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { CodeQualityStore, type CodeQualityFinding } from "./code-quality-store.js";

function freshStore(): CodeQualityStore {
  const dir = mkdtempSync(join(tmpdir(), "oxplow-code-quality-"));
  return new CodeQualityStore(dir);
}

const finding = (overrides: Partial<CodeQualityFinding> = {}): CodeQualityFinding => ({
  path: "src/foo.ts",
  startLine: 1,
  endLine: 10,
  kind: "complexity",
  metricValue: 5,
  extra: null,
  ...overrides,
});

describe("CodeQualityStore", () => {
  let store: CodeQualityStore;
  beforeEach(() => {
    store = freshStore();
  });

  test("startScan returns an id and a row in 'running' state", () => {
    const scanId = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    expect(scanId).toBeGreaterThan(0);
    const scans = store.listScans({ streamId: "s1" });
    expect(scans).toHaveLength(1);
    expect(scans[0]!.tool).toBe("lizard");
    expect(scans[0]!.scope).toBe("codebase");
    expect(scans[0]!.status).toBe("running");
    expect(scans[0]!.completed_at).toBeNull();
  });

  test("completeScan inserts findings and flips status to completed", () => {
    const scanId = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(scanId, [
      finding({ path: "a.ts", metricValue: 12 }),
      finding({ path: "b.ts", metricValue: 3 }),
    ]);
    const scans = store.listScans({ streamId: "s1" });
    expect(scans[0]!.status).toBe("completed");
    expect(scans[0]!.completed_at).not.toBeNull();
    const findings = store.listLatestFindings({ streamId: "s1" });
    expect(findings).toHaveLength(2);
    expect(findings.map((f) => f.path).sort()).toEqual(["a.ts", "b.ts"]);
  });

  test("failScan flips status to failed and records error_message", () => {
    const scanId = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.failScan(scanId, "lizard not on PATH");
    const scans = store.listScans({ streamId: "s1" });
    expect(scans[0]!.status).toBe("failed");
    expect(scans[0]!.error_message).toBe("lizard not on PATH");
    expect(scans[0]!.completed_at).not.toBeNull();
  });

  test("listLatestFindings returns only findings from the most recent completed scan per (stream, tool, scope)", () => {
    const oldScan = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(oldScan, [finding({ path: "old.ts" })]);
    const newScan = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(newScan, [finding({ path: "new.ts" })]);
    const findings = store.listLatestFindings({ streamId: "s1", tool: "lizard" });
    expect(findings.map((f) => f.path)).toEqual(["new.ts"]);
  });

  test("listLatestFindings can filter by tool", () => {
    const lizardScan = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(lizardScan, [finding({ path: "lizard.ts", kind: "complexity" })]);
    const jscpdScan = store.startScan({ streamId: "s1", tool: "jscpd", scope: "codebase" });
    store.completeScan(jscpdScan, [finding({ path: "dup.ts", kind: "duplicate-block" })]);
    expect(store.listLatestFindings({ streamId: "s1", tool: "lizard" }).map((f) => f.path)).toEqual([
      "lizard.ts",
    ]);
    expect(store.listLatestFindings({ streamId: "s1", tool: "jscpd" }).map((f) => f.path)).toEqual([
      "dup.ts",
    ]);
    expect(store.listLatestFindings({ streamId: "s1" }).map((f) => f.path).sort()).toEqual([
      "dup.ts",
      "lizard.ts",
    ]);
  });

  test("listLatestFindings ignores running scans", () => {
    const completed = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(completed, [finding({ path: "done.ts" })]);
    store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" }); // still running
    const findings = store.listLatestFindings({ streamId: "s1", tool: "lizard" });
    expect(findings.map((f) => f.path)).toEqual(["done.ts"]);
  });

  test("listLatestFindings can filter by paths", () => {
    const scan = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(scan, [
      finding({ path: "a.ts" }),
      finding({ path: "b.ts" }),
      finding({ path: "c.ts" }),
    ]);
    const findings = store.listLatestFindings({ streamId: "s1", paths: ["a.ts", "c.ts"] });
    expect(findings.map((f) => f.path).sort()).toEqual(["a.ts", "c.ts"]);
  });

  test("retention prunes scans beyond keepLast per (stream, tool, scope)", () => {
    const store = new CodeQualityStore(mkdtempSync(join(tmpdir(), "oxplow-cq-retain-")), undefined, {
      keepLast: 2,
    });
    const ids: number[] = [];
    for (let i = 0; i < 4; i++) {
      const id = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
      store.completeScan(id, [finding({ path: `f${i}.ts` })]);
      ids.push(id);
    }
    const scans = store.listScans({ streamId: "s1" });
    expect(scans).toHaveLength(2);
    expect(scans.map((s) => s.id).sort((a, b) => a - b)).toEqual(ids.slice(2).sort((a, b) => a - b));
    // findings for pruned scans should be gone too
    const findings = store.listLatestFindings({ streamId: "s1", tool: "lizard" });
    expect(findings.map((f) => f.path)).toEqual(["f3.ts"]);
  });

  test("scope is independent for retention (codebase and diff retained separately)", () => {
    const store = new CodeQualityStore(mkdtempSync(join(tmpdir(), "oxplow-cq-scope-")), undefined, {
      keepLast: 1,
    });
    const cb = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(cb, [finding({ path: "cb.ts" })]);
    const diff = store.startScan({ streamId: "s1", tool: "lizard", scope: "diff", baseRef: "main" });
    store.completeScan(diff, [finding({ path: "diff.ts" })]);
    const scans = store.listScans({ streamId: "s1" });
    expect(scans).toHaveLength(2);
  });

  test("subscribe fires on start, complete, and fail", () => {
    const events: string[] = [];
    store.subscribe((c) => events.push(`${c.kind}:${c.tool}`));
    const a = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(a, [finding()]);
    const b = store.startScan({ streamId: "s1", tool: "jscpd", scope: "codebase" });
    store.failScan(b, "boom");
    expect(events).toEqual([
      "started:lizard",
      "completed:lizard",
      "started:jscpd",
      "failed:jscpd",
    ]);
  });

  test("findings include extra_json when provided", () => {
    const scan = store.startScan({ streamId: "s1", tool: "lizard", scope: "codebase" });
    store.completeScan(scan, [
      finding({ path: "a.ts", extra: { functionName: "doThing", parameters: 4 } }),
    ]);
    const findings = store.listLatestFindings({ streamId: "s1" });
    expect(findings[0]!.extra).toEqual({ functionName: "doThing", parameters: 4 });
  });
});
