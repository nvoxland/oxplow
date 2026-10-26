/// The prompt catalog (`v_page_answer`) as pages use it: what to ask about a
/// kind of ref (the Assistant's suggestion chips).

/** One question a person can ask, with who offers it. */
export interface CatalogPrompt {
  prompt: string;
  /** A ref kind (`file`, `commit`, `effort`): a page for one offers it. */
  about: string | null;
  source: { kind: "extension"; name: string };
}

/** The prompts a page for a ref of `kind` suggests. */
export function promptsAbout(catalog: CatalogPrompt[], kind: string): CatalogPrompt[] {
  return catalog.filter((p) => p.about === kind);
}
