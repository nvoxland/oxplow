/// The pages extensions add to the launcher (P6.D1): their lenses' and
/// their `pages:`. What they offer to run is a command — declared with a
/// `ui`, offered like oxplow's own (`commandOffers`).
import type { Extension } from "../tauri-bridge/generated/bindings.js";
import { lensDirectoryEntries } from "../lens/lensModel.js";
import { extPageRef } from "../tabs/pageRefs.js";
import type { PageDirectoryEntry } from "./RailHud/sections.js";

export function launcherPages(extensions: Extension[]): PageDirectoryEntry[] {
  const pages = lensDirectoryEntries(extensions);
  for (const ext of extensions) {
    if (!ext.enabled) continue;
    for (const page of ext.pages) {
      const ref = extPageRef(ext.name, page.id);
      pages.push({ id: ref.id, label: page.title, ref, category: page.category, keywords: `${ext.name} ${page.id}` });
    }
  }
  return pages;
}
