import { expect, test } from "bun:test";

import type { SignInCompletion } from "../tauri-bridge/generated/bindings.js";
import { createSignInHost, type SignInDeps } from "./hostSignIn.js";

/** The shell, the browser and the core, faked: `redirects` are what the
 *  shell catches, in order; a listener's wait ends when it is stopped. */
function world(overrides: Partial<SignInDeps> = {}, redirects: string[] = ["/callback?code=a"]) {
  const log: string[] = [];
  const waiters = new Map<number, { reject(e: Error): void }>();
  let nextListener = 0;
  let nextSignIn = 0;
  const deps: SignInDeps = {
    canCatch: () => true,
    redirectPort: async () => 8124,
    listen: async (port) => {
      const id = ++nextListener;
      log.push(`listen ${port}`);
      return { id, port: port ?? 5555 };
    },
    awaitRedirect: (id) => {
      const next = redirects.shift();
      if (next !== undefined) return Promise.resolve(next);
      return new Promise((_, reject) => waiters.set(id, { reject }));
    },
    answer: async (_id, outcome) => void log.push(`answer ${outcome.outcome}`),
    stop: async (id) => {
      log.push(`stop ${id}`);
      waiters.get(id)?.reject(new Error("stopped"));
    },
    openBrowser: async (url) => void log.push(`open ${url}`),
    begin: async (_i, _n, port) => {
      log.push(`begin ${port}`);
      return { url: "https://svc/auth", signIn: ++nextSignIn };
    },
    complete: async () => ({ outcome: "signed_in" }) as SignInCompletion,
    cancel: async (_i, _n, signIn) => void log.push(`cancel ${signIn}`),
    ...overrides,
  };
  return { host: createSignInHost(deps), log };
}

test("a sign-in listens on the registered port, opens the browser and ends signed in", async () => {
  const w = world();
  expect(await w.host.signIn("t/x", "TOKEN")).toEqual({ state: "signed_in" });
  expect(w.log).toEqual(["listen 8124", "begin 8124", "open https://svc/auth", "answer signed_in", "stop 1"]);
});

test("a redirect that isn't its own is refused and the wait goes on", async () => {
  const answers = ["not_this_sign_in", "signed_in"];
  const w = world({ complete: async () => ({ outcome: answers.shift(), reason: "r" }) as SignInCompletion }, ["/a", "/b"]);
  expect(await w.host.signIn("t/x", "TOKEN")).toEqual({ state: "signed_in" });
  expect(w.log.filter((l) => l.startsWith("answer"))).toEqual(["answer not_this_sign_in", "answer signed_in"]);
});

test("a failure from the core is how it ended, and the core isn't told to cancel what is over", async () => {
  const w = world({ complete: async () => ({ outcome: "failed", error: "denied" }) as SignInCompletion });
  expect(await w.host.signIn("t/x", "TOKEN")).toEqual({ state: "failed", error: "denied" });
  expect(w.log.some((l) => l.startsWith("cancel"))).toBe(false);
});

test("cancelling ends the wait cancelled, stopping the shell and telling the core", async () => {
  const w = world({}, []);
  const waiting = w.host.signIn("t/x", "TOKEN");
  await new Promise((r) => setTimeout(r, 5));
  await w.host.cancel("t/x", "TOKEN");
  expect(await waiting).toEqual({ state: "cancelled" });
  expect(w.log.slice(-2)).toEqual(["stop 1", "cancel 1"]);
});

test("a newer sign-in for the credential replaces the one under way", async () => {
  const w = world({}, []);
  const first = w.host.signIn("t/x", "TOKEN");
  await new Promise((r) => setTimeout(r, 5));
  const second = w.host.signIn("t/x", "TOKEN");
  expect(await first).toEqual({ state: "cancelled" });
  await new Promise((r) => setTimeout(r, 5));
  await w.host.closeAll();
  expect(await second).toEqual({ state: "cancelled" });
  expect(w.log.filter((l) => l.startsWith("listen")).length).toBe(2);
});

test("without the desktop app a sign-in is refused INVALID before anything starts", async () => {
  const w = world({ canCatch: () => false });
  const refused = await w.host.signIn("t/x", "TOKEN").catch((e: unknown) => e);
  expect((refused as { code?: string }).code).toBe("INVALID");
  expect(w.log).toEqual([]);
});

test("a browser that won't open ends the attempt: the shell and the core forget it", async () => {
  const w = world({ openBrowser: async () => Promise.reject(new Error("no browser")) });
  const refused = await w.host.signIn("t/x", "TOKEN").catch((e: unknown) => e);
  expect(String(refused)).toContain("no browser");
  expect(w.log.slice(-2)).toEqual(["stop 1", "cancel 1"]);
});
