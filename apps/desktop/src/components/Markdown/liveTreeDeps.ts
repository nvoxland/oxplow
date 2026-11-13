/** A component frame's markdown lookups in the app (`markdownTree`'s
 *  `TreeDeps`): the stores' ids read first, and a ref link's title by the
 *  viewer's rule over the same title cache. */
import { loadRefTitle } from "../../refTitleCache.js";
import { loadRecordStores } from "../../recordStoresLoader.js";
import { refLinkTitle } from "./markdownLinks.js";
import type { TreeDeps } from "./markdownTree.js";

export async function liveTreeDeps(): Promise<TreeDeps> {
  await loadRecordStores();
  return {
    refTitle: async (ref, text) => refLinkTitle(ref, text, ref ? await loadRefTitle(ref) : { title: null, status: "loading" }),
  };
}
