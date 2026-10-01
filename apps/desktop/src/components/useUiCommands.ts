import { useMemo } from "react";

import { useExtensions } from "../extensionsStore.js";
import type { UiCommand } from "../tauri-bridge/generated/bindings.js";

/** The enabled extensions' `ui.commands` for `streamId` (from the shared
 *  extensions store, so it follows their changes). */
export function useUiCommands(streamId: string | null): UiCommand[] {
  const exts = useExtensions(streamId);
  return useMemo(() => (exts ?? []).filter((e) => e.enabled).flatMap((e) => e.ui.commands), [exts]);
}
