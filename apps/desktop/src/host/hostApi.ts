/** A host's API as a device's page uses it (`crates/oxplow-host/src/api.rs`,
 *  `.context/host.md`): pair, list the projects, hold or release one, forget
 *  this device. `fetch` is a test's stand-in. */

export type DeviceKind = "desktop" | "tablet" | "phone";

export type PairedDevice = { id: string; name: string; kind: DeviceKind; paired_at: number };

export type HostProject = {
  id: string;
  name: string;
  state: "starting" | "running" | "stopping" | "stopped" | "crashed";
  holds: number;
  held_by_you: boolean;
  message?: string;
};

type Fetch = typeof fetch;

/** A host's refusal: its own words, and its status (401: this device's
 *  token is no longer good). */
export class HostRefusal extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

async function refusal(answer: Response): Promise<HostRefusal> {
  const body = (await answer.json().catch(() => null)) as { error?: string } | null;
  return new HostRefusal(body?.error ?? `the host answered ${answer.status}`, answer.status);
}

/** Pair this device with `code` (as the owner shows it, any case). */
export async function pairDevice(
  host: string,
  code: string,
  name: string,
  kind: DeviceKind,
  doFetch: Fetch = fetch,
): Promise<{ device: PairedDevice; token: string }> {
  const answer = await doFetch(`${host}/host/pair`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code: code.trim(), name: name.trim(), kind }),
  });
  if (!answer.ok) throw await refusal(answer);
  return (await answer.json()) as { device: PairedDevice; token: string };
}

function auth(token: string): Record<string, string> {
  return { Authorization: `Bearer ${token}` };
}

export async function listProjects(host: string, token: string, doFetch: Fetch = fetch): Promise<HostProject[]> {
  const answer = await doFetch(`${host}/host/projects`, { headers: auth(token) });
  if (!answer.ok) throw await refusal(answer);
  const projects = (await answer.json()) as Array<Omit<HostProject, "state"> & { state: HostProject["state"] }>;
  return projects;
}

/** This device holds `project` (its daemon starts) or lets go of it. */
export async function setHold(
  host: string,
  token: string,
  project: string,
  held: boolean,
  doFetch: Fetch = fetch,
): Promise<void> {
  const answer = await doFetch(`${host}/host/holds`, {
    method: held ? "POST" : "DELETE",
    headers: { ...auth(token), "Content-Type": "application/json" },
    body: JSON.stringify({ project }),
  });
  if (!answer.ok) throw await refusal(answer);
}

/** This device forgets itself: its token stops working. */
export async function forgetDevice(host: string, token: string, doFetch: Fetch = fetch): Promise<void> {
  const answer = await doFetch(`${host}/host/me`, { method: "DELETE", headers: auth(token) });
  if (!answer.ok && answer.status !== 401) throw await refusal(answer);
}
