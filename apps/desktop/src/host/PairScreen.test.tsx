import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { shellStorage } from "../shellStorage.js";
import { PairScreen } from "./PairScreen.js";

afterEach(() => {
  cleanup();
  shellStorage.removeItem("oxplow.hostToken");
});

// Enter pairs; the token and the host's id for the device are kept, and the
// page loads again.
test("a good code keeps the device's token and id, then loads again", async () => {
  let reloaded = false;
  const asked: string[] = [];
  render(
    <PairScreen
      pair={async (_host, code, name) => {
        asked.push(`${code}|${name}`);
        return { device: { id: "dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427", name, kind: "tablet", paired_at: 0 }, token: "tok" };
      }}
      done={() => (reloaded = true)}
    />,
  );
  fireEvent.change(screen.getByTestId("pair-code"), { target: { value: "abcd-efgh" } });
  fireEvent.change(screen.getByTestId("pair-name"), { target: { value: "Kitchen" } });
  fireEvent.submit(screen.getByTestId("pair-code").closest("form")!);
  await waitFor(() => expect(reloaded).toBe(true));
  expect(asked).toEqual(["ABCD-EFGH|Kitchen"]);
  expect(shellStorage.getItem("oxplow.hostToken")).toBe("tok");
  expect(shellStorage.getItem("oxplow.device")).toBe("dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427");
});

test("a wrong code says why and keeps what was typed", async () => {
  render(
    <PairScreen
      pair={async () => {
        throw new Error("that code is wrong or has expired");
      }}
      done={() => {}}
    />,
  );
  fireEvent.change(screen.getByTestId("pair-code"), { target: { value: "WXYZ-2345" } });
  fireEvent.submit(screen.getByTestId("pair-code").closest("form")!);
  await waitFor(() => expect(screen.getByTestId("pair-error").textContent).toContain("wrong"));
  expect((screen.getByTestId("pair-code") as HTMLInputElement).value).toBe("WXYZ-2345");
  expect(shellStorage.getItem("oxplow.hostToken")).toBeNull();
});
