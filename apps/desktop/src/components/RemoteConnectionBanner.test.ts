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

  // a refused token is its own state — not a drop to wait out —
  // and nothing the socket does afterwards changes it.
  test("a refused token shows refused, and stays", () => {
    expect(nextBannerState("hidden", "refused")).toBe("refused");
    expect(nextBannerState("down", "refused")).toBe("refused");
    expect(nextBannerState("refused", "down")).toBe("refused");
    expect(nextBannerState("refused", "up")).toBe("refused");
  });

  // The app that runs the host quit: its own words, until the host is back.
  test("other devices turned off says so, and their return is a restore", () => {
    expect(nextBannerState("hidden", "closed")).toBe("closed");
    expect(nextBannerState("closed", "down")).toBe("closed");
    expect(nextBannerState("closed", "up")).toBe("restored");
  });

  test("a host that ended says so, and its return is a restore", () => {
    expect(nextBannerState("hidden", "ended")).toBe("ended");
    expect(nextBannerState("down", "ended")).toBe("ended");
    expect(nextBannerState("ended", "down")).toBe("ended");
    expect(nextBannerState("ended", "up")).toBe("restored");
  });

  test("flap sequence lands on restored", () => {
    const events: Array<"up" | "down"> = ["up", "down", "up", "down", "up"];
    const final = events.reduce<BannerState>((s, e) => nextBannerState(s, e), "hidden");
    expect(final).toBe("restored");
  });
});
