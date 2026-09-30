/// What extensions add to the launcher (P6.D1): their lenses, and their
/// manifest `launcher:` entries — a ref opens as a page; a command runs as
/// the person; a prompt goes into the agent's input, never sent. See
/// `.context/extensions.md` → "Launcher entries".
import type { Extension, LauncherCategory, LauncherTarget } from "../tauri-bridge/generated/bindings.js";
import { lensDirectoryEntries } from "../lens/lensModel.js";
import { extPageRef, refFromTabId } from "../tabs/pageRefs.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

/** A launcher entry that isn't a page: a command or a prompt. */
export interface LauncherAction {
  id: string;
  extension: string;
  label: string;
  category: LauncherCategory;
  target: Exclude<LauncherTarget, { kind: "ref" }>;
}

export function launcherDirectory(extensions: Extension[]): {
  pages: PageDirectoryEntry[];
  actions: LauncherAction[];
} {
  const pages = lensDirectoryEntries(extensions);
  const actions: LauncherAction[] = [];
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pages) {
      const ref = extPageRef(ext.name, page.id);
      pages.push({ id: ref.id, label: page.title, ref, category: page.category, keywords: `${ext.name} ${page.id}` });
    }
    for (const entry of ext.launcher) {
      const { target } = entry;
      if (target.kind === "ref") {
        const ref = refFromTabId(target.ref);
        if (ref) {
          pages.push({ id: ref.id, label: entry.label, ref, category: entry.category, keywords: ext.name });
        }
        continue;
      }
      actions.push({ id: `${ext.name}:${entry.label}`, extension: ext.name, label: entry.label, category: entry.category, target });
    }
  }
  return { pages, actions };
}
