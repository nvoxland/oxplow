import { expect, test } from "bun:test";
import { askToPair, collectPairing, HostRefusal, listProjects, setHold } from "./hostApi.js";

type Call = { url: string; init?: RequestInit };

function fakeFetch(answer: { status: number; body: unknown }, calls: Call[]): typeof fetch {
  return (async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(answer.body), { status: answer.status });
  }) as unknown as typeof fetch;
}

// Asking to pair sends JSON (the host refuses anything else) with this
// device's name and kind, and answers the code to show and the ask's secret.
test("asking to pair sends the name as JSON and answers the code to show", async () => {
  const calls: Call[] = [];
  const asked = await askToPair(
    "https://mac.ts.net:7400",
    " Tab S9 ",
    "tablet",
    fakeFetch({ status: 200, body: { request: "secret", code: "ABCD-EFGH", expires_in: 600 } }, calls),
  );
  expect(asked).toEqual({ request: "secret", code: "ABCD-EFGH", expires_in: 600 });
  expect(calls[0]!.url).toBe("https://mac.ts.net:7400/host/pair");
  expect((calls[0]!.init!.headers as Record<string, string>)["Content-Type"]).toBe("application/json");
  expect(JSON.parse(String(calls[0]!.init!.body))).toEqual({ name: "Tab S9", kind: "tablet" });
});

// Collecting the ask: none while the owner has not typed the code (202),
// the device and its token once they did, the host's words once it ended.
test("collecting an ask waits, then answers the device, or says it ended", async () => {
  const calls: Call[] = [];
  expect(await collectPairing("h", "secret", fakeFetch({ status: 202, body: { waiting: true } }, calls))).toBeNull();
  expect(calls[0]!.url).toBe("h/host/pair/secret");
  const paired = await collectPairing(
    "h",
    "secret",
    fakeFetch({ status: 200, body: { device: { id: "dev-1", name: "Tab S9", kind: "tablet" }, token: "t" } }, []),
  );
  expect(paired?.token).toBe("t");
  const ended = collectPairing("h", "secret", fakeFetch({ status: 404, body: { error: "this ask to pair ended" } }, []));
  await expect(ended).rejects.toThrow("ended");
  expect(await ended.catch((e: unknown) => (e instanceof HostRefusal ? e.status : null))).toBe(404);
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
