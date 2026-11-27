import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { shellStorage } from "../shellStorage.js";
import { PairScreen } from "./PairScreen.js";

afterEach(() => {
  cleanup();
  shellStorage.removeItem("oxplow.hostToken");
});

const DEVICE = { id: "dev-1b4e28ba-2fa1-11d2-883f-0016d3cca427", name: "Kitchen", kind: "tablet" as const, paired_at: 0 };

// The device asks with its name and shows the code for the owner to type
// on the computer; once they did, it keeps its token and id and loads again.
test("asking shows the code to type on the computer, then keeps the token once approved", async () => {
  let reloaded = false;
  const asked: string[] = [];
  const answers: Array<{ device: typeof DEVICE; token: string } | null> = [null, { device: DEVICE, token: "tok" }];
  render(
    <PairScreen
      ask={async (_host, name) => {
        asked.push(name);
        return { request: "secret", code: "ABCD-EFGH", expires_in: 600 };
      }}
      collect={async (_host, request) => {
        asked.push(`collect ${request}`);
        return answers.shift() ?? null;
      }}
      pause={0}
      done={() => (reloaded = true)}
    />,
  );
  fireEvent.change(screen.getByTestId("pair-name"), { target: { value: "Kitchen" } });
  fireEvent.submit(screen.getByTestId("pair-name").closest("form")!);
  expect((await screen.findByTestId("pair-shown-code")).textContent).toBe("ABCD-EFGH");
  await waitFor(() => expect(reloaded).toBe(true));
  expect(asked).toEqual(["Kitchen", "collect secret", "collect secret"]);
  expect(shellStorage.getItem("oxplow.hostToken")).toBe("tok");
  expect(shellStorage.getItem("oxplow.device")).toBe(DEVICE.id);
});

// An ask the owner did not answer in time says so; asking again is one
// press away, with the name kept.
test("an ask that ended says why and offers asking again", async () => {
  render(
    <PairScreen
      ask={async () => ({ request: "secret", code: "ABCD-EFGH", expires_in: 600 })}
      collect={async () => {
        throw new Error("this ask to pair ended (after ten minutes); ask again");
      }}
      pause={0}
      done={() => {}}
    />,
  );
  fireEvent.change(screen.getByTestId("pair-name"), { target: { value: "Kitchen" } });
  fireEvent.submit(screen.getByTestId("pair-name").closest("form")!);
  await waitFor(() => expect(screen.getByTestId("pair-error").textContent).toContain("ended"));
  expect((screen.getByTestId("pair-name") as HTMLInputElement).value).toBe("Kitchen");
  expect(screen.queryByTestId("pair-shown-code")).toBeNull();
  expect(shellStorage.getItem("oxplow.hostToken")).toBeNull();
});
