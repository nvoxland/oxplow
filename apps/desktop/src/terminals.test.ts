import { expect, test } from "bun:test";

import { terminalsFromResult } from "./terminals.js";

test("the rows of v_terminal are the stream's terminals, by id and URI", () => {
  const rows = terminalsFromResult({
    columns: ["id", "uri", "title"],
    rows: [
      [3, "oxplow:oxplow/terminal?id=trm3", "Terminal 1"],
      [5, "oxplow:oxplow/terminal?id=trm5", "build"],
    ],
    truncated: false,
  } as never);
  expect(rows).toEqual([
    { id: "trm3", uri: "oxplow:oxplow/terminal?id=trm3", title: "Terminal 1" },
    { id: "trm5", uri: "oxplow:oxplow/terminal?id=trm5", title: "build" },
  ]);
});
