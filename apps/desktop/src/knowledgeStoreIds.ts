/**
 * The knowledge stores (`v_model_store` of the knowledge model), as the app
 * last read them (`knowledgeStores.ts` keeps this current): what makes
 * `[[<store>:<id>]]` one of their pages in markdown, and which store a bare
 * `[[slug]]` belongs to — the first whose id pattern (`fills.id_pattern`)
 * matches it, as a task store's does for `tsk42`. No imports, so the markdown
 * link code — which the fixture harness bundles too — can read it without
 * React or the app's transport.
 */
export interface KnowledgeStoreId {
  store: string;
  /** How its page ids look in text, a regex matched whole; null when it doesn't say. */
  idPattern: string | null;
}

let stores: KnowledgeStoreId[] = [];

export function enabledKnowledgeStoreIds(): string[] {
  return stores.map((s) => s.store);
}

export function setKnowledgeStoreIds(next: KnowledgeStoreId[]): void {
  stores = next;
}

/** The store a bare `[[id]]` names: the first whose id pattern matches it whole. */
export function knowledgeStoreOfBareId(id: string): string | null {
  for (const s of stores) {
    if (!s.idPattern) continue;
    try {
      if (new RegExp(`^(?:${s.idPattern})$`).test(id)) return s.store;
    } catch {
      // A pattern that isn't a regex says nothing.
    }
  }
  return null;
}
