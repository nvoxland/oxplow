/**
 * The enabled knowledge stores' ids, as the app last read them
 * (`knowledgeStores.ts` keeps this current): what makes `[[<store>:<id>]]`
 * one of their pages in markdown. No imports, so the markdown link code —
 * which the fixture harness bundles too — can read it without React or the
 * app's transport.
 */
/** oxwiki's store id: a bare `[[slug]]` is one of its pages. */
export const OXWIKI = "oxwiki";

let ids: string[] = [];

export function enabledKnowledgeStoreIds(): string[] {
  return ids;
}

export function setKnowledgeStoreIds(next: string[]): void {
  ids = next;
}
