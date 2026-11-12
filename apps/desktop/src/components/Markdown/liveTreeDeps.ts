/** A component frame's markdown lookups in the app (`markdownTree`'s
 *  `TreeDeps`): the stores' ids from their profiles, and a ref link's
 *  title by the viewer's rule over the same title caches. */
import { loadExtensionRefTitle } from "../../resources.js";
import { loadKnowledgeRef } from "../../knowledgeTitleCache.js";
import { loadTaskRef } from "../../taskTitleCache.js";
import { loadTaskTrackerProfiles } from "../../useTaskTrackerProfiles.js";
import { taskRefOfMention } from "../../tasks.js";
import { refLinkTitle } from "./markdownLinks.js";
import type { TreeDeps } from "./markdownTree.js";

export async function liveTreeDeps(): Promise<TreeDeps> {
  const profile = await loadTaskTrackerProfiles();
  return {
    mentionRef: (id) => taskRefOfMention(profile, id),
    refTitle: async (names, text) => {
      const [knowledge, item, extensionTitle] = await Promise.all([
        names.knowledgeRef ? loadKnowledgeRef(names.knowledgeRef) : NONE,
        names.itemRef ? loadTaskRef(names.itemRef) : NONE,
        names.extensionRef ? loadExtensionRefTitle(names.extensionRef) : null,
      ]);
      return refLinkTitle(names, text, { knowledge, item, extensionTitle });
    },
  };
}

const NONE = { title: null, status: "loading" as const };
