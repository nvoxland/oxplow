import { useCallback, useEffect, useRef, useState } from "react";
import { deleteWikiPage, writeWikiPage } from "../../api.js";
import { readWikiPage, type WikiPageSummary } from "../../knowledge.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import type { Reads } from "../../tauri-bridge/generated/bindings.js";
import { recordOpError } from "../opErrorsStore.js";

export interface WikiPageController {
  summary: WikiPageSummary | null;
  body: string;
  draft: string;
  setDraft(value: string): void;
  draftInitialized: boolean;
  editing: boolean;
  notFound: boolean;
  loadError: string | null;
  isDirty: boolean;
  enterEdit(): void;
  enterView(): void;
  save(): Promise<void>;
  revert(): void;
  create(): Promise<void>;
  remove(): Promise<void>;
}

export function useWikiPageController(slug: string, onClosed: () => void): WikiPageController {
  const [summary, setSummary] = useState<WikiPageSummary | null>(null);
  const [body, setBody] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [draftInitialized, setDraftInitialized] = useState(false);

  const [reads, setReads] = useState<Reads>(NO_READS);

  const refresh = useCallback(async () => {
    try {
      const { page, reads } = await readWikiPage(slug);
      setSummary(page?.summary ?? null);
      setBody(page?.body ?? "");
      setNotFound(page === null);
      setLoadError(null);
      setReads(reads);
    } catch (error) {
      setLoadError(String(error));
      setNotFound(false);
    }
  }, [slug]);

  useEffect(() => {
    void refresh();
    setEditing(false);
  }, [refresh]);

  // Re-read when a model the read read changes (the page row, its body,
  // its refs) — the one rerun rule every model read uses.
  useRerunOnChange(reads, () => void refresh());

  useEffect(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
  }, [body, draftInitialized]);

  useEffect(() => {
    setDraftInitialized(false);
  }, [slug]);

  const enterEdit = useCallback(() => {
    if (!draftInitialized) {
      setDraft(body);
      setDraftInitialized(true);
    }
    setEditing(true);
  }, [body, draftInitialized]);

  const enterView = useCallback(() => {
    setEditing(false);
  }, []);

  const revert = useCallback(() => {
    setDraft(body);
  }, [body]);

  const save = useCallback(async () => {
    try {
      await writeWikiPage(slug, draft);
      setBody(draft);
    } catch (error) {
      recordOpError({
        label: `Save wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [slug, draft]);

  const create = useCallback(async () => {
    const seed = `# ${slug}\n\n`;
    try {
      await writeWikiPage(slug, seed);
      setNotFound(false);
      setBody(seed);
      setDraft(seed);
      setDraftInitialized(true);
      setEditing(true);
    } catch (error) {
      recordOpError({
        label: `Create wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [slug]);

  // The page's Delete asks inline first; that is the confirmation the
  // destructive `oxplow.knowledge.delete_page` asks for.
  const remove = useCallback(async () => {
    try {
      await deleteWikiPage(slug, true);
      onClosed();
    } catch (error) {
      recordOpError({
        label: `Delete wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [slug, onClosed]);

  return {
    summary,
    body,
    draft,
    setDraft,
    draftInitialized,
    editing,
    notFound,
    loadError,
    isDirty: draft !== body,
    enterEdit,
    enterView,
    save,
    revert,
    create,
    remove,
  };
}
