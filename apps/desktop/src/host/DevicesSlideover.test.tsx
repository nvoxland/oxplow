import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { DevicesView, HostSettings } from "../tauri-bridge/generated/bindings.js";
import { countdown, DevicesSlideover, type DevicesApi } from "./DevicesSlideover.js";

afterEach(() => cleanup());

function fake(over: Partial<DevicesView> = {}) {
  const calls: string[] = [];
  let settings: HostSettings = { allowOtherDevices: false, address: "", names: [], tlsCert: null, tlsKey: null };
  const api: DevicesApi = {
    hostDevices: async () => ({
      thisDevice: "Studio",
      devices: [{ id: "dev-1", name: "Tab S9", kind: "tablet", pairedAt: 0, connected: true }],
      settings,
      settingsApplied: false,
      openOnDevice: ["https://mac.ts.net:7400/"],
      public: { state: "off" },
      hostError: null,
      ...over,
    }),
    hostPairingCode: async () => ({ code: "ABCD-EFGH", expiresInSeconds: 600 }),
    hostRenameDevice: async (id, name) => {
      calls.push(`rename ${id} ${name}`);
    },
    hostRemoveDevice: async (id) => {
      calls.push(`remove ${id}`);
    },
    setHostSettings: async (next) => {
      calls.push(`save ${JSON.stringify(next)}`);
      settings = next;
    },
    hostTailscale: async () => ({ address: "100.64.0.1", name: "mac.ts.net" }),
  };
  return { api, calls };
}

test("a code shows with where to open it on the device", async () => {
  const { api } = fake();
  render(<DevicesSlideover open onClose={() => {}} api={api} />);
  fireEvent.click(await screen.findByTestId("devices-pair"));
  const shown = await screen.findByTestId("devices-code");
  expect(shown.textContent).toContain("ABCD-EFGH");
  expect(shown.textContent).toContain("https://mac.ts.net:7400/");
  expect(screen.getByTestId("devices-off")).toBeTruthy();
});

test("a device is renamed in place and removed after a confirm", async () => {
  const { api, calls } = fake();
  render(<DevicesSlideover open onClose={() => {}} api={api} />);
  await screen.findByTestId("device-dev-1");
  fireEvent.click(screen.getByText("Tab S9"));
  const name = screen.getByRole("textbox", { name: "Device name" });
  fireEvent.change(name, { target: { value: "Kitchen" } });
  fireEvent.keyDown(name, { key: "Enter" });
  await waitFor(() => expect(calls).toContain("rename dev-1 Kitchen"));
  fireEvent.click(screen.getByText("Remove"));
  fireEvent.click(screen.getByTestId("device-remove-dev-1-confirm"));
  await waitFor(() => expect(calls).toContain("remove dev-1"));
});

// Save is off until something differs; Tailscale fills the address and
// name; Discard puts the saved values back.
test("the settings save only when changed", async () => {
  const { api, calls } = fake();
  render(<DevicesSlideover open onClose={() => {}} api={api} />);
  const save = (await screen.findByTestId("devices-save")) as HTMLButtonElement;
  expect(save.disabled).toBe(true);
  fireEvent.click(screen.getByTestId("devices-allow"));
  fireEvent.click(screen.getByTestId("devices-tailscale"));
  await waitFor(() => expect((screen.getByTestId("devices-address") as HTMLInputElement).value).toBe("100.64.0.1"));
  expect(save.disabled).toBe(false);
  fireEvent.click(save);
  await waitFor(() => expect(calls.some((c) => c.startsWith("save "))).toBe(true));
  const saved = JSON.parse(calls.find((c) => c.startsWith("save "))!.slice(5)) as HostSettings;
  expect(saved).toEqual({ allowOtherDevices: true, address: "100.64.0.1", names: ["mac.ts.net"], tlsCert: null, tlsKey: null });
});

test("Escape in the settings puts the saved values back", async () => {
  const { api, calls } = fake();
  render(<DevicesSlideover open onClose={() => {}} api={api} />);
  const address = (await screen.findByTestId("devices-address")) as HTMLInputElement;
  fireEvent.change(address, { target: { value: "100.64.0.9" } });
  fireEvent.keyDown(address, { key: "Escape" });
  expect(address.value).toBe("");
  expect(calls).toEqual([]);
});

// A host that did not answer (a setting that cannot work) still shows the
// settings, to fix; a listener that failed says why.
test("the settings show when the host does not answer", async () => {
  const { api } = fake({ hostError: "the host did not start", devices: [], public: null });
  render(<DevicesSlideover open onClose={() => {}} api={api} />);
  expect((await screen.findByTestId("devices-host-error")).textContent).toContain("did not start");
  expect(screen.getByTestId("devices-settings")).toBeTruthy();
  cleanup();
  const failed = fake({ public: { state: "failed", error: "bind 100.64.0.1:7400 failed" } });
  render(<DevicesSlideover open onClose={() => {}} api={failed.api} />);
  expect((await screen.findByTestId("devices-public-failed")).textContent).toContain("bind");
});

test("a countdown reads as minutes and seconds", () => {
  expect(countdown(600)).toBe("10:00");
  expect(countdown(61.9)).toBe("1:01");
  expect(countdown(-3)).toBe("0:00");
});
