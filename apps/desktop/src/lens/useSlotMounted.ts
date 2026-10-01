import { useExtensions } from "../extensionsStore.js";
import { slotExtensions } from "./lensModel.js";

/** Whether any enabled extension mounts a lens in `slot` — for a page
 *  that gives a slot a region of its own (a side column) only when
 *  something fills it. */
export function useSlotMounted(slot: string, streamId: string | null): boolean {
  return slotExtensions(useExtensions(streamId) ?? [], slot).length > 0;
}
