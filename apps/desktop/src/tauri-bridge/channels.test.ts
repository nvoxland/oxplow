import { describe, expect, test } from "bun:test";
import {
  CHANNEL_ROUTING,
  EVENT_CHANNELS,
  type ListenChannel,
  type ChannelRouting,
} from "./channels";
import { listen } from "./transport";

describe("CHANNEL_ROUTING registry", () => {
  test("classifies exactly the EVENT_CHANNELS values as multiplexed", () => {
    // Drift guard: the multiplexed set must equal the daemon-bridged
    // channels (EVENT_CHANNELS), so the WS demux table and the routing
    // classification can't disagree.
    const multiplexed = Object.entries(CHANNEL_ROUTING)
      .filter(([, routing]) => routing === "multiplexed")
      .map(([name]) => name)
      .sort();
    expect(multiplexed).toEqual([...Object.values(EVENT_CHANNELS)].sort());
  });

  test("every entry carries a valid routing classification", () => {
    const valid: ChannelRouting[] = ["multiplexed", "shellLocal"];
    for (const routing of Object.values(CHANNEL_ROUTING)) {
      expect(valid).toContain(routing);
    }
  });

  test("the native menu is shell-local", () => {
    expect(CHANNEL_ROUTING["menu:command"]).toBe("shellLocal");
  });

  test("listen() rejects an unclassified channel at compile time", () => {
    // The AC: a channel not in the registry is a type error, not a
    // silent runtime fall-through. @ts-expect-error fails the build if
    // the call ever starts type-checking (i.e. the union widened).
    // @ts-expect-error "not:a:channel" is not a ListenChannel
    const bad: Promise<unknown> = listen("not:a:channel", () => {});
    void bad;

    // A declared channel type-checks fine.
    const ok: ListenChannel = "menu:command";
    expect(ok).toBe("menu:command");
  });
});
