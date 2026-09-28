import { expect, test } from "bun:test";

import type { SchemaEntity } from "../tauri-bridge/generated/bindings.js";
import { entityRows, entitySummary, programRow } from "./dataSectionModel.js";

const entity = (name: string, owner: string, available = true): SchemaEntity => ({
  name,
  owner,
  available,
  description: `${name} rows`,
  columns: [],
  relations: [],
});

test("entityRows puts core first, formats counts and flags unsynced entities", () => {
  const rows = entityRows(
    [entity("v_github_pr", "github", false), entity("v_task", "core"), entity("v_commit", "core"), entity("v_linear_issue", "linear")],
    [
      { name: "v_task", rows: 12345 },
      { name: "v_commit", rows: 0 },
      { name: "v_linear_issue", rows: 7 },
      { name: "v_github_pr", rows: null },
    ],
  );
  expect(rows.map((r) => r.name)).toEqual(["v_commit", "v_task", "v_github_pr", "v_linear_issue"]);
  expect(rows[1]!.rows).toBe(new Intl.NumberFormat().format(12345));
  expect(rows[0]!.rows).toBe("0");
  expect(rows[2]!.rows).toBe("Not synced yet");
  expect(rows[2]!.available).toBe(false);
  expect(entitySummary(rows)).toBe("4 entities · 2 from extensions");
  expect(entitySummary(rows.slice(0, 1))).toBe("1 entity");
});

test("an entity missing from the counts reads as a dash, not zero", () => {
  expect(entityRows([entity("v_task", "core")], [])[0]!.rows).toBe("—");
});

test("programRow says what runs and whether it will", () => {
  const m = programRow({ kind: "plugin", name: "acme.parse", program: "tools/parse.sh", args: ["--x"], env: [], approved: false });
  expect(m.label).toBe("Collection plugin acme.parse");
  expect(m.command).toBe("tools/parse.sh --x");
  expect(m.status).toBe("Not approved: it won't run");
  expect(m.approveTitle).toContain("tools/parse.sh --x");
  expect(programRow({ kind: "gauge", name: "repo.n", program: "t.sh", args: [], env: [], approved: true }).status).toBe(
    "Approved on this machine",
  );
  // An ACP agent shows the env it runs with, since that's part of what's approved.
  const agent = programRow({ kind: "acp-agent", name: "mine", program: "tools/agent", args: ["--acp"], env: ["MODE=fast"], approved: false });
  expect([agent.label, agent.command]).toEqual(["ACP agent mine", "MODE=fast tools/agent --acp"]);
});
