import { newId } from "./newId.js";
import { shellStorage } from "./shellStorage.js";
import { readWindowContext } from "./windowContext.js";

export type Device = { id: string; name: string; kind: "desktop" | "tablet" | "phone" };

const DEVICE_ID = /^dev-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

type Sources = {
  injected: { id: string; name: string } | null;
  stored: { get(): string | null; set(id: string): void };
  coarsePointer: boolean;
  generate?: () => string;
};

/** Who this client is to the daemon. The Mac app's shell names its device;
 *  a browser keeps one id in storage, so a reload is the same device. Pure
 *  over its sources; exported for tests. */
export function createDevice(src: Sources): Device {
  if (src.injected) return { id: src.injected.id, name: src.injected.name, kind: "desktop" };
  let id = src.stored.get();
  if (!id || !DEVICE_ID.test(id)) {
    id = `dev-${(src.generate ?? newId)()}`;
    src.stored.set(id);
  }
  return { id, name: "Browser", kind: src.coarsePointer ? "tablet" : "desktop" };
}

function coarsePointer(): boolean {
  try {
    return globalThis.matchMedia?.("(pointer: coarse)").matches ?? false;
  } catch {
    return false;
  }
}

let device: Device | null = null;

/** The device, made once per page load. No API/transport dependency: the
 *  transport uses it on every request. */
export function getDevice(): Device {
  return (device ??= createDevice({
    injected: readWindowContext()?.device ?? null,
    stored: {
      get: () => shellStorage.getItem("oxplow.device"),
      set: (id) => shellStorage.setItem("oxplow.device", id),
    },
    coarsePointer: coarsePointer(),
  }));
}
