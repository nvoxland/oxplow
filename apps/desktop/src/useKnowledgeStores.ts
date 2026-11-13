import { useEffect, useState } from "react";
import type { KnowledgeStore } from "./knowledge.js";
import { knowledgeStores, onKnowledgeStores } from "./knowledgeStores.js";

/** The enabled knowledge stores, re-rendering when they change. */
export function useKnowledgeStores(): KnowledgeStore[] {
  const [value, setValue] = useState<KnowledgeStore[]>(() => knowledgeStores());
  useEffect(() => {
    const update = () => setValue(knowledgeStores());
    const stop = onKnowledgeStores(update);
    update();
    return stop;
  }, []);
  return value;
}
