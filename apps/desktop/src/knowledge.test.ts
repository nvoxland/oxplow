import { expect, test } from "bun:test";

import type { SearchHit, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { freshnessFromResult, knowledgeChanged, pagesFromResult, searchHitsOf } from "./knowledge.js";

const result = (columns: string[], rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({ columns, rows, truncated: false, reads: { models: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

test("the index reads v_knowledge_page rows", () => {
  const pages = pagesFromResult(
    result(
      ["ref", "slug", "title", "excerpt", "updated_at", "stale_ref_count", "outbound_refs"],
      [["wiki:auth", "auth", "Auth", "How login…", "2026-09-30T00:00:00Z", 2, '["file:src/a.rs","wiki:other"]']],
    ),
  );
  expect(pages).toEqual([
    {
      ref: "wiki:auth",
      slug: "auth",
      title: "Auth",
      excerpt: "How login…",
      updated_at: "2026-09-30T00:00:00Z",
      stale_ref_count: 2,
      outbound_refs: ["file:src/a.rs", "wiki:other"],
    },
  ]);
});

test("freshness reads v_knowledge_ref rows", () => {
  expect(
    freshnessFromResult(
      result(
        ["path", "pinned_snapshot_id", "pinned_vcs_rev", "pinned_vcs_rev_exact", "latest_snapshot_id", "stale"],
        [["src/a.rs", 3, "abc", 1, 5, 1], ["src/b.rs", null, null, 0, null, 0]],
      ),
    ),
  ).toEqual([
    { path: "src/a.rs", pinned_snapshot_id: 3, pinned_vcs_rev: "abc", pinned_vcs_rev_exact: true, latest_snapshot_id: 5, stale: true },
    { path: "src/b.rs", pinned_snapshot_id: null, pinned_vcs_rev: null, pinned_vcs_rev_exact: false, latest_snapshot_id: null, stale: false },
  ]);
});

test("title and body search is the site search's wiki hits", () => {
  const hits = [
    { kind: "wiki", ref_id: "auth", stream_id: null, title: "Auth", snippet: "…login…", score: 1 },
    { kind: "task", ref_id: "tsk1", stream_id: null, title: "x", snippet: "", score: 1 },
  ] as SearchHit[];
  expect(searchHitsOf(hits)).toEqual([{ slug: "auth", title: "Auth", snippet: "…login…" }]);
});

test("knowledge reads re-run when a knowledge model changes", () => {
  expect(knowledgeChanged({ kind: "modelsChanged", models: ["v_knowledge_body"] })).toBe(true);
  expect(knowledgeChanged({ kind: "modelsChanged", models: ["v_task"] })).toBe(false);
});
