/** A component frame's markdown lookups in the app (`markdownTree`'s
 *  `TreeDeps`): the active list's ids from its profile, and a ref link's
 *  title by the viewer's rule over the same title caches. */
import { loadExtensionRefTitle } from "../../resources.js";
import { loadWikiRef } from "../../wikiTitleCache.js";
import { loadWorkItemRef } from "../../workItemTitleCache.js";
import { loadWorkListProfile } from "../../useWorkListProfile.js";
import { workItemRefOfMention } from "../../workItems.js";
import { refLinkTitle } from "./markdownLinks.js";
import type { TreeDeps } from "./markdownTree.js";

export async function liveTreeDeps(): Promise<TreeDeps> {
  const profile = await loadWorkListProfile();
  return {
    mentionRef: (id) => workItemRefOfMention(profile, id),
    refTitle: async (names, text) => {
      const [wiki, item, extensionTitle] = await Promise.all([
        names.internalSlug ? loadWikiRef(names.internalSlug) : NONE,
        names.itemRef ? loadWorkItemRef(names.itemRef) : NONE,
        names.extensionRef ? loadExtensionRefTitle(names.extensionRef) : null,
      ]);
      return refLinkTitle(names, text, { wiki, item, extensionTitle });
    },
  };
}

const NONE = { title: null, status: "loading" as const };
