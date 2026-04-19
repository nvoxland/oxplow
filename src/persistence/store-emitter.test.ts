import { describe, expect, test } from "bun:test";
import { StoreEmitter } from "./store-emitter.js";

describe("StoreEmitter", () => {
  test("subscribe returns an unsubscribe function", () => {
    const e = new StoreEmitter<number>("test");
    const seen: number[] = [];
    const off = e.subscribe((n) => seen.push(n));
    e.emit(1);
    e.emit(2);
    off();
    e.emit(3);
    expect(seen).toEqual([1, 2]);
  });

  test("snapshots listeners during emit so a self-unsubscribing listener doesn't skip later ones", () => {
    const e = new StoreEmitter<string>("test");
    const seen: string[] = [];
    const offA = e.subscribe((v) => { seen.push(`a:${v}`); offA(); });
    e.subscribe((v) => seen.push(`b:${v}`));
    e.emit("hi");
    expect(seen).toEqual(["a:hi", "b:hi"]);
  });

  test("a throwing listener does not stop subsequent listeners", () => {
    const e = new StoreEmitter<string>("test");
    const seen: string[] = [];
    e.subscribe(() => { throw new Error("boom"); });
    e.subscribe((v) => seen.push(v));
    e.emit("after-boom");
    expect(seen).toEqual(["after-boom"]);
  });
});
