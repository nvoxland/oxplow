import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { RemoteHostView } from "../tauri-bridge/generated/bindings.js";
import { RemoteHostsSection, type RemoteHostsApi } from "./RemoteHostsSection.js";

afterEach(() => cleanup());

function fake(hosts: RemoteHostView[]) {
  const calls: string[] = [];
  const api: RemoteHostsApi = {
    remoteHosts: async () => hosts,
    connectHost: async (address, code) => {
      calls.push(`connect ${address} ${code}`);
    },
    openRemoteProject: async (origin, project, close) => {
      calls.push(`open ${origin} ${project} ${close}`);
    },
    forgetRemoteHost: async (origin) => {
      calls.push(`forget ${origin}`);
    },
  };
  return { api, calls };
}

const server: RemoteHostView = {
  origin: "https://server.ts.net:7400",
  deviceName: "Studio",
  version: "0.8.0",
  otherVersion: true,
  projects: [{ id: "prj-1", name: "api", state: "running", holds: 1, heldByYou: false }],
  error: null,
};

// Enter connects with the address and the code as typed.
test("a host is connected with its address and a code", async () => {
  const { api, calls } = fake([]);
  render(<RemoteHostsSection api={api} />);
  fireEvent.change(screen.getByTestId("remote-host-address"), { target: { value: "server.ts.net:7400" } });
  fireEvent.change(screen.getByTestId("remote-host-code"), { target: { value: "abcd-efgh" } });
  fireEvent.submit(screen.getByTestId("remote-host-code").closest("form")!);
  await waitFor(() => expect(calls).toEqual(["connect server.ts.net:7400 ABCD-EFGH"]));
});

test("a host's projects open here, and another version is said", async () => {
  const { api, calls } = fake([server]);
  render(<RemoteHostsSection api={api} />);
  fireEvent.click(await screen.findByTestId("remote-project-open-prj-1"));
  await waitFor(() => expect(calls).toEqual(["open https://server.ts.net:7400 prj-1 true"]));
  expect(screen.getByTestId(`remote-host-${server.origin}`).textContent).toContain("another version");
});
