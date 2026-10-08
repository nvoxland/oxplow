import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

import type { HarnessListing } from "../tauri-bridge/generated/bindings.js";
import { NewThreadSessionSetting } from "./NewThreadSessionSetting.js";

afterEach(cleanup);

const harnesses: HarnessListing[] = [
  { id: "claude", title: "Claude", chat: false, enabled: true, settings: [] },
  { id: "codex", title: "Codex", chat: false, enabled: false, settings: [] },
];

/** Ask, No session, then each agent a session can start with; a change
 *  saves at once. */
test("new threads start with ask, none or an agent, saved on change", async () => {
  const saved: string[] = [];
  const { getByTestId } = render(
    <NewThreadSessionSetting
      harnesses={harnesses}
      read={async () => "none"}
      save={async (choice) => {
        saved.push(choice);
      }}
    />,
  );
  const select = getByTestId("settings-new-thread-session") as HTMLSelectElement;
  await waitFor(() => expect(select.value).toBe("none"));
  expect(Array.from(select.options).map((o) => [o.value, o.textContent])).toEqual([
    ["ask", "Ask (the session picker)"],
    ["none", "No session"],
    ["claude", "Claude"],
  ]);
  fireEvent.change(select, { target: { value: "claude" } });
  await waitFor(() => expect(saved).toEqual(["claude"]));
  expect(select.value).toBe("claude");
});

/** A remembered agent that can't start any more still shows, so the
 *  person sees why new threads ask. */
test("a remembered agent that isn't available shows as such", async () => {
  const { getByTestId } = render(
    <NewThreadSessionSetting harnesses={harnesses} read={async () => "codex"} save={async () => {}} />,
  );
  const select = getByTestId("settings-new-thread-session") as HTMLSelectElement;
  await waitFor(() => expect(select.value).toBe("codex"));
  expect(select.selectedOptions[0]!.textContent).toBe("codex (not available: new threads ask)");
});
