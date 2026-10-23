import { afterAll, expect, mock, test } from "bun:test";

// An answer recorded after the window started watching its thread is
// shown (opened); the ones before it aren't reopened.

const realApi = { ...(await import("./api.js")) };
afterAll(() => {
  mock.module("./api.js", () => realApi);
});
let rows: [number, string][] = [[1, "oxplow:oxplow/answer?id=1"]];
/** Ids the daemon counts as recorded in the last ten seconds. */
let recent = new Set<number>();
let emit: (event: unknown) => void = () => {};
mock.module("./api.js", () => ({
  ...realApi,
  querySql: async (sql: string, params: unknown[]) => {
    if (sql.includes("max(id)")) return { columns: ["m"], rows: [[Math.max(0, ...rows.filter((r) => !recent.has(r[0])).map((r) => r[0]))]], truncated: false };
    const after = Number(params[1]);
    return { columns: ["id", "uri"], rows: rows.filter((r) => r[0] > after), truncated: false };
  },
  subscribeOxplowEvents: (cb: (event: unknown) => void) => {
    emit = cb;
    return () => {};
  },
}));
const { watchShownAnswers } = await import("./shownAnswers.js");

test("only answers recorded after watching began are shown, each once", async () => {
  const shown: string[] = [];
  const stop = watchShownAnswers("thr1", (uri) => shown.push(uri));
  await new Promise((r) => setTimeout(r, 0));
  rows = [...rows, [2, "oxplow:oxplow/answer?id=2"]];
  emit({ kind: "modelsChanged", models: ["v_thread_answer"] });
  await new Promise((r) => setTimeout(r, 0));
  emit({ kind: "modelsChanged", models: ["v_thread_answer"] });
  await new Promise((r) => setTimeout(r, 0));
  expect(shown).toEqual(["oxplow:oxplow/answer?id=2"]);
  stop();
});

test("one recorded just before watching began (the window still starting) is shown", async () => {
  rows = [[1, "oxplow:oxplow/answer?id=1"], [2, "oxplow:oxplow/answer?id=2"]];
  recent = new Set([2]);
  const shown: string[] = [];
  const stop = watchShownAnswers("thr1", (uri) => shown.push(uri));
  await new Promise((r) => setTimeout(r, 0));
  await new Promise((r) => setTimeout(r, 0));
  expect(shown).toEqual(["oxplow:oxplow/answer?id=2"]);
  stop();
});
