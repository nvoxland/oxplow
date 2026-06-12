import { describe, expect, test } from "bun:test";

import type { SearchHit } from "../api.js";
import { dedupeSiteHits } from "./quickOpenResults.js";

function hit(kind: string, refId: string): SearchHit {
  return {
    kind,
    ref_id: refId,
    stream_id: null,
    title: refId,
    snippet: "…",
    score: -1,
  };
}

describe("dedupeSiteHits", () => {
  test("keeps body hits for pages and non-matching files", () => {
    const out = dedupeSiteHits(
      [hit("wiki", "architecture-overview"), hit("file", "src/other.rs"), hit("task", "12")],
      new Set(["src/main.rs"]),
    );
    expect(out.map((h) => h.ref_id)).toEqual(["architecture-overview", "src/other.rs", "12"]);
  });

  test("drops file hits whose path already matched by filename", () => {
    const out = dedupeSiteHits(
      [hit("file", "src/main.rs"), hit("wiki", "w")],
      new Set(["src/main.rs"]),
    );
    expect(out.map((h) => h.ref_id)).toEqual(["w"]);
  });

  test("non-file kinds are never deduped against paths", () => {
    const out = dedupeSiteHits([hit("wiki", "src/main.rs")], new Set(["src/main.rs"]));
    expect(out).toHaveLength(1);
  });
});
