import { useCallback, useEffect, useState } from "react";
import { useDraft } from "../../tabs/useViewParam.js";
import { deleteWikiPage, writeWikiPage } from "../../api.js";
import { readWikiPage, type WikiPageSummary } from "../../knowledge.js";
import { NO_READS, useRerunOnChange } from "../../lens/lensRerun.js";
import type { Reads } from "../../tauri-bridge/generated/bindings.js";
import { recordOpError } from "../../opErrors.js";

export interface WikiPageController {
  summary: WikiPageSummary | null;
  body: string;
  draft: string;
  setDraft(value: string): void;
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
  // The edit is the step's history state: Back and a restart bring an
  // unsaved edit back. No draft yet means the draft is the saved body.
  const [editing, setEditing] = useDraft("editing", false);
  const [draftText, setDraft, clearDraft] = useDraft<string | null>("draft", null);
  const draft = draftText ?? body;
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);

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
  }, [refresh]);

  // Re-read when a model the read read changes (the page row, its body,
  // its refs) — the one rerun rule every model read uses.
  useRerunOnChange(reads, () => void refresh());

  const enterEdit = useCallback(() => setEditing(true), [setEditing]);

  const enterView = useCallback(() => {
    setEditing(false);
  }, [setEditing]);

  const revert = clearDraft;

  const save = useCallback(async () => {
    try {
      await writeWikiPage(slug, draft);
      setBody(draft);
      clearDraft();
    } catch (error) {
      recordOpError({
        label: `Save wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [slug, draft, clearDraft]);

  const create = useCallback(async () => {
    const seed = `# ${slug}\n\n`;
    try {
      await writeWikiPage(slug, seed);
      setNotFound(false);
      setBody(seed);
      clearDraft();
      setEditing(true);
    } catch (error) {
      recordOpError({
        label: `Create wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [slug, clearDraft, setEditing]);

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
