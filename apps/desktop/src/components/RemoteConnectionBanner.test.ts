import { describe, expect, test } from "bun:test";

import { nextBannerState, type BannerState } from "./RemoteConnectionBanner.js";

describe("nextBannerState", () => {
  test("initial up (first connect) stays hidden", () => {
    expect(nextBannerState("hidden", "up")).toBe("hidden");
  });

  test("drop shows the down banner", () => {
    expect(nextBannerState("hidden", "down")).toBe("down");
  });

  test("recovery after a drop shows restored", () => {
    expect(nextBannerState("down", "up")).toBe("restored");
  });

  test("restored is sticky across further ups until dismissed", () => {
    expect(nextBannerState("restored", "up")).toBe("restored");
  });

  test("a second drop from restored returns to down", () => {
    expect(nextBannerState("restored", "down")).toBe("down");
  });

  // tsk971: a refused token is its own state — not a drop to wait out —
  // and nothing the socket does afterwards changes it.
  test("a refused token shows refused, and stays", () => {
    expect(nextBannerState("hidden", "refused")).toBe("refused");
    expect(nextBannerState("down", "refused")).toBe("refused");
    expect(nextBannerState("refused", "down")).toBe("refused");
    expect(nextBannerState("refused", "up")).toBe("refused");
  });

  test("flap sequence lands on restored", () => {
    const events: Array<"up" | "down"> = ["up", "down", "up", "down", "up"];
    const final = events.reduce<BannerState>((s, e) => nextBannerState(s, e), "hidden");
    expect(final).toBe("restored");
  });
});
