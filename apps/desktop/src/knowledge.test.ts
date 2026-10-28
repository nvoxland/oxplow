import { expect, test } from "bun:test";

import type { SearchHit, SqlQueryResult } from "./tauri-bridge/generated/bindings.js";
import { knowledgeIsNone, NO_KNOWLEDGE_STORE, pagesFromResult } from "./knowledge.js";

const result = (columns: string[], rows: SqlQueryResult["rows"]): SqlQueryResult =>
  ({ columns, rows, truncated: false, reads: { datasets: [], tables: [], measures: [] }, freshness: {} }) as unknown as SqlQueryResult;

test("the index reads v_knowledge_page rows", () => {
  const pages = pagesFromResult(
    result(
      ["uri", "slug", "title", "excerpt", "updated_at", "stale_ref_count", "outbound_uris"],
      [["oxplow:oxplow/wiki?slug=auth", "auth", "Auth", "How login…", "2026-09-30T00:00:00Z", 2, '["oxplow:oxplow/file?path=src/a.rs","oxplow:oxplow/wiki?slug=other"]']],
    ),
  );
  expect(pages).toEqual([
    {
      uri: "oxplow:oxplow/wiki?slug=auth",
      slug: "auth",
      title: "Auth",
      excerpt: "How login…",
      updated_at: "2026-09-30T00:00:00Z",
      stale_ref_count: 2,
      outbound_uris: ["oxplow:oxplow/file?path=src/a.rs", "oxplow:oxplow/wiki?slug=other"],
    },
  ]);
});

// Knowledge is none when its active row is `none`; nothing listed yet is not none.
test("knowledge is none only when the active row says so", () => {
  const row = (provider: string, active: boolean) =>
    ({ capability: "knowledge", provider, extension: null, features: {}, fields: [], idPattern: null, active }) as never;
  expect(knowledgeIsNone([row("oxplow", false), row("none", true)])).toBe(true);
  expect(knowledgeIsNone([row("oxplow", true), row("none", false)])).toBe(false);
  expect(knowledgeIsNone([])).toBe(false);
  expect(NO_KNOWLEDGE_STORE).toBe("No knowledge store — choose one in Settings → Capabilities");
});
