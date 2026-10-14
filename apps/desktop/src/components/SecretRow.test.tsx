import { afterEach, expect, mock, test } from "bun:test";
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";

// A host secret reference an extension's provider declares: presence, Set
// as a masked inline field (Enter commits, Escape cancels), Revoke behind
// a confirm. The value goes to the keychain and never comes back.

const realApi = await import("../api.js");
const calls: Array<{ op: string; args: unknown[] }> = [];
let present = false;
mock.module("../api.js", () => ({
  ...realApi,
  extensionSecretIsSet: async (...args: unknown[]) => {
    calls.push({ op: "is_set", args });
    return present;
  },
  saveExtensionSecret: async (...args: unknown[]) => {
    calls.push({ op: "save", args });
    present = true;
  },
  revokeExtensionSecret: async (...args: unknown[]) => {
    calls.push({ op: "revoke", args });
    present = false;
  },
}));
const { SecretRow } = await import("./ExtensionsSection.js");

const reference = { schemaVersion: 1, id: "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b" } as const;
const credential = { provider: "remote", name: "OXPLOW_MCP_BEARER", reference };
const id = "remote-ext-remote-OXPLOW_MCP_BEARER";
const row = () => render(<SecretRow extension="remote-ext" credential={credential} />);

afterEach(() => {
  calls.length = 0;
  present = false;
  cleanup();
});

test("a secret that isn't set says so and offers Set", async () => {
  const view = row();
  await waitFor(() => expect(view.getByTestId(`secret-status-${id}`).textContent).toBe("Not set"));
  expect(view.getByTestId(`secret-set-${id}`).textContent).toContain("Set…");
  expect(view.queryByTestId(`secret-revoke-${id}-trigger`)).toBeNull();
});

test("Enter in the masked field saves the value, and the row then offers Rotate", async () => {
  const view = row();
  await waitFor(() => view.getByTestId(`secret-status-${id}`));
  fireEvent.click(view.getByTestId(`secret-set-${id}`));
  const input = view.getByTestId(`secret-set-${id}`) as HTMLInputElement;
  expect(input.type).toBe("password");
  fireEvent.change(input, { target: { value: "s3cret" } });
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() => expect(calls.some((c) => c.op === "save")).toBe(true));
  expect(calls.find((c) => c.op === "save")!.args).toEqual(["remote-ext", reference, "s3cret"]);
  await waitFor(() => expect(view.getByTestId(`secret-status-${id}`).textContent).toBe("Set"));
  expect(view.getByTestId(`secret-set-${id}`).textContent).toContain("Rotate…");
});

test("Escape cancels without saving", async () => {
  const view = row();
  await waitFor(() => view.getByTestId(`secret-status-${id}`));
  fireEvent.click(view.getByTestId(`secret-set-${id}`));
  const input = view.getByTestId(`secret-set-${id}`);
  fireEvent.change(input, { target: { value: "nope" } });
  fireEvent.keyDown(input, { key: "Escape" });
  expect(calls.some((c) => c.op === "save")).toBe(false);
});
