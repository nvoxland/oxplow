import { afterEach, expect, test } from "bun:test";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { RemoteHostView } from "../tauri-bridge/generated/bindings.js";
import { RemoteHostsSection, type RemoteHostsApi } from "./RemoteHostsSection.js";

afterEach(() => cleanup());

function fake(hosts: RemoteHostView[], answers: Array<boolean | Error> = [true]) {
  const calls: string[] = [];
  const api: RemoteHostsApi = {
    remoteHosts: async () => {
      calls.push("list");
      return hosts;
    },
    askHost: async (address) => {
      calls.push(`ask ${address}`);
      return { origin: "https://server.ts.net:7400", request: "secret-1", code: "ABCD-EFGH", expiresIn: 600 };
    },
    collectHost: async (origin, request) => {
      calls.push(`collect ${origin} ${request}`);
      const next = answers.shift() ?? false;
      if (next instanceof Error) throw next;
      return next;
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

// Connect asks the host there and shows the code to type there; it waits
// until the person there types it, then lists the host.
test("connecting shows the code to type on that computer and waits for it", async () => {
  const { api, calls } = fake([], [false, true]);
  render(<RemoteHostsSection api={api} pause={0} />);
  fireEvent.change(screen.getByTestId("remote-host-address"), { target: { value: "server.ts.net:7400" } });
  fireEvent.submit(screen.getByTestId("remote-host-address").closest("form")!);
  expect((await screen.findByTestId("remote-host-ask-code")).textContent).toBe("ABCD-EFGH");
  expect(screen.getByTestId("remote-host-ask").textContent).toContain("oxplow host pair ABCD-EFGH");
  await waitFor(() => expect(screen.queryByTestId("remote-host-ask") === null).toBe(true));
  expect(calls.filter((c) => c !== "list")).toEqual([
    "ask server.ts.net:7400",
    "collect https://server.ts.net:7400 secret-1",
    "collect https://server.ts.net:7400 secret-1",
  ]);
  expect(calls.at(-1)).toBe("list");
});

// An ask that ended (ten minutes) says so; Cancel stops waiting.
test("an ask that ended says so, and one can be cancelled", async () => {
  const ended = fake([], [new Error("this ask to pair ended (after ten minutes); ask again")]);
  render(<RemoteHostsSection api={ended.api} pause={0} />);
  fireEvent.change(screen.getByTestId("remote-host-address"), { target: { value: "s:7400" } });
  fireEvent.submit(screen.getByTestId("remote-host-address").closest("form")!);
  expect((await screen.findByTestId("remote-host-error")).textContent).toContain("ended");
  expect(screen.queryByTestId("remote-host-ask")).toBeNull();
  cleanup();

  const waiting = fake([], []);
  render(<RemoteHostsSection api={waiting.api} pause={0} />);
  fireEvent.change(screen.getByTestId("remote-host-address"), { target: { value: "s:7400" } });
  fireEvent.submit(screen.getByTestId("remote-host-address").closest("form")!);
  fireEvent.click(await screen.findByTestId("remote-host-ask-cancel"));
  expect(screen.queryByTestId("remote-host-ask")).toBeNull();
});

test("a host's projects open here, and another version is said", async () => {
  const { api, calls } = fake([server]);
  render(<RemoteHostsSection api={api} pause={0} />);
  fireEvent.click(await screen.findByTestId("remote-project-open-prj-1"));
  await waitFor(() => expect(calls.filter((c) => c !== "list")).toEqual(["open https://server.ts.net:7400 prj-1 true"]));
  expect(screen.getByTestId(`remote-host-${server.origin}`).textContent).toContain("another version");
});
