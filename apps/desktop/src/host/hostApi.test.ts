import { expect, test } from "bun:test";
import { HostRefusal, listProjects, pairDevice, setHold } from "./hostApi.js";

type Call = { url: string; init?: RequestInit };

function fakeFetch(answer: { status: number; body: unknown }, calls: Call[]): typeof fetch {
  return (async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(answer.body), { status: answer.status });
  }) as unknown as typeof fetch;
}

// Pairing sends JSON (the host refuses anything else) with the code as
// typed, and answers the device and its token.
test("pairing sends the code as JSON and answers the device", async () => {
  const calls: Call[] = [];
  const paired = await pairDevice(
    "https://mac.ts.net:7400",
    " abcd-efgh ",
    "Tab S9",
    "tablet",
    fakeFetch({ status: 200, body: { device: { id: "dev-1", name: "Tab S9", kind: "tablet" }, token: "t" } }, calls),
  );
  expect(paired.token).toBe("t");
  expect(calls[0]!.url).toBe("https://mac.ts.net:7400/host/pair");
  expect((calls[0]!.init!.headers as Record<string, string>)["Content-Type"]).toBe("application/json");
  expect(JSON.parse(String(calls[0]!.init!.body))).toEqual({ code: "abcd-efgh", name: "Tab S9", kind: "tablet" });
});

test("a refusal carries the host's own words", async () => {
  const wrong = pairDevice("h", "x", "n", "tablet", fakeFetch({ status: 401, body: { error: "that code is wrong" } }, []));
  await expect(wrong).rejects.toThrow("that code is wrong");
  const status = await wrong.catch((e: unknown) => (e instanceof HostRefusal ? e.status : null));
  expect(status).toBe(401);
});

test("projects and holds carry the device's token", async () => {
  const calls: Call[] = [];
  const list = await listProjects("h", "tok", fakeFetch({ status: 200, body: [{ id: "prj-1", name: "a" }] }, calls));
  expect(list[0]!.id).toBe("prj-1");
  await setHold("h", "tok", "prj-1", false, fakeFetch({ status: 200, body: {} }, calls));
  expect(calls[1]!.init!.method).toBe("DELETE");
  for (const call of calls) {
    expect((call.init!.headers as Record<string, string>).Authorization).toBe("Bearer tok");
  }
});
