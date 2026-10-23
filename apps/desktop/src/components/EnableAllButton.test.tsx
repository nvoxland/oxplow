import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// Enable All Disabled runs `oxplow.contribution.enable_all` as the person.

const realApi = await import("../api.js");
const ran: Array<{ name: string; input: unknown; confirmed: boolean }> = [];
mock.module("../api.js", () => ({
  ...realApi,
  runCommand: async (name: string, input: unknown, confirmed: boolean) => {
    ran.push({ name, input, confirmed });
    return { result: null };
  },
}));
const { EnableAllButton } = await import("./EnableAllButton.js");

afterEach(() => {
  ran.length = 0;
  cleanup();
});

test("Enable All Disabled runs oxplow.contribution.enable_all", async () => {
  const all = render(<EnableAllButton count={34} />);
  expect(all.getByTestId("extension-enable-all-all").textContent).toBe("Enable All Disabled (34)");
  fireEvent.click(all.getByTestId("extension-enable-all-all"));
  await waitFor(() => expect(ran).toHaveLength(1));
  expect(ran[0]).toEqual({ name: "oxplow.contribution.enable_all", input: {}, confirmed: false });
});

test("Enable All Disabled is not offered when nothing is disabled", () => {
  const view = render(<EnableAllButton count={0} />);
  expect(view.queryByTestId("extension-enable-all-all")).toBeNull();
});
