import { afterEach, expect, mock, test } from "bun:test";
import { act, cleanup, renderHook } from "@testing-library/react";

import type { Extension, Renderer } from "../tauri-bridge/generated/bindings.js";
import { createRenderersCache, useRenderers } from "./useRenderers.js";

afterEach(cleanup);

const r = (page: string, over: Partial<Renderer> = {}): Renderer => ({ page, title: page, default: false, ...over });
const extensions = () => [{ name: "x" }] as unknown as Extension[];
const FILE = "oxplow:oxplow/file?path=a.ts";

/** An rpc stub that answers each call by hand, so a test sees the pending state. */
function stub() {
  const calls: { uri: string; streamId: string | null; answer: (l: Renderer[]) => void; fail: (e: Error) => void }[] = [];
  const rpc = mock((uri: string, streamId: string | null) => new Promise<Renderer[]>((answer, fail) => calls.push({ uri, streamId, answer, fail })));
  return { rpc, calls };
}

test("a list is asked once per identity and stream, however often or with whatever view", async () => {
  const { rpc, calls } = stub();
  const cache = createRenderersCache(rpc);
  const exts = extensions();
  const a = cache.prefetch(exts, FILE, "s1");
  const b = cache.prefetch(exts, `${FILE}#oxplow.renderer=acme/blame`, "s1");
  expect(rpc).toHaveBeenCalledTimes(1);
  // The view is the window's: the daemon is asked about the resource alone.
  expect(calls[0]!.uri).toBe(FILE);
  expect(cache.peek(exts, FILE, "s1")).toBeUndefined();
  calls[0]!.answer([r("editor", { default: true }), r("viewer")]);
  expect((await a).map((x) => x.page)).toEqual(["editor", "viewer"]);
  expect(await b).toBe(await a);
  expect(cache.peek(exts, FILE, "s1")?.map((x) => x.page)).toEqual(["editor", "viewer"]);
  // Another stream is another ask: what a file holds depends on its stream.
  void cache.prefetch(exts, FILE, "s2");
  expect(rpc).toHaveBeenCalledTimes(2);
});

test("reloaded extensions drop every list: a deploy can change pages and what they accept", async () => {
  const { rpc, calls } = stub();
  const cache = createRenderersCache(rpc);
  const first = extensions();
  const asked = cache.prefetch(first, FILE, null);
  calls[0]!.answer([r("editor")]);
  await asked;
  expect(cache.peek(first, FILE, null)).toBeDefined();
  const second = extensions();
  expect(cache.peek(second, FILE, null)).toBeUndefined();
  const again = cache.prefetch(second, FILE, null);
  expect(rpc).toHaveBeenCalledTimes(2);
  calls[1]!.answer([r("editor"), r("blame")]);
  await again;
  expect(cache.peek(second, FILE, null)?.length).toBe(2);
});

test("an answer that arrives after the extensions reloaded is not kept", async () => {
  const { rpc, calls } = stub();
  const cache = createRenderersCache(rpc);
  const first = extensions();
  const late = cache.prefetch(first, FILE, null);
  const second = extensions();
  expect(cache.peek(second, FILE, null)).toBeUndefined();
  calls[0]!.answer([r("stale")]);
  await late;
  expect(cache.peek(second, FILE, null)).toBeUndefined();
});

test("a failed ask is an empty list, so the fallback page draws the URI", async () => {
  const { rpc, calls } = stub();
  const cache = createRenderersCache(rpc);
  const exts = extensions();
  const asked = cache.prefetch(exts, FILE, null);
  calls[0]!.fail(new Error("daemon is away"));
  expect(await asked).toEqual([]);
  expect(cache.peek(exts, FILE, null)).toEqual([]);
});

test("the hook asks for each uri once the extensions load, and re-renders when one settles", async () => {
  const { rpc, calls } = stub();
  const cache = createRenderersCache(rpc);
  const exts = extensions();
  const { result, rerender } = renderHook(
    ({ e }: { e: Extension[] | null }) => useRenderers(e, [FILE, "oxplow:oxplow/commit?sha=abc"], "s1", cache),
    { initialProps: { e: null as Extension[] | null } },
  );
  expect(rpc).not.toHaveBeenCalled();
  expect(result.current(FILE)).toBeUndefined();
  rerender({ e: exts });
  expect(rpc).toHaveBeenCalledTimes(2);
  const first = result.current;
  expect(first(FILE)).toBeUndefined();
  await act(async () => {
    calls[0]!.answer([r("editor")]);
    await Promise.resolve();
  });
  // A new lookup: what memoizes over it (the tabs) recomputes.
  expect(result.current).not.toBe(first);
  expect(result.current(FILE)?.map((x) => x.page)).toEqual(["editor"]);
  expect(result.current("oxplow:oxplow/commit?sha=abc")).toBeUndefined();
});
