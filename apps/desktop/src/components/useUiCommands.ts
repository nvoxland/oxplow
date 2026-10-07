import { useMemo } from "react";

import { useExtensions } from "../extensionsStore.js";
import type { UiCommand } from "../tauri-bridge/generated/bindings.js";

/** The enabled extensions' `ui.commands` (from the shared extensions
 *  store, so it follows their changes). */
export function useUiCommands(): UiCommand[] {
  const exts = useExtensions();
  return useMemo(() => (exts ?? []).filter((e) => e.enabled).flatMap((e) => e.ui.commands), [exts]);
}
