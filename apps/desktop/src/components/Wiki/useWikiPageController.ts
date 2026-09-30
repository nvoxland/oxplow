import { useCallback, useEffect, useRef, useState } from "react";
import { deleteWikiPage, subscribeOxplowEvents, writeWikiPage, type Stream } from "../../api.js";
import { knowledgeChanged, readWikiPage, type WikiPageSummary } from "../../knowledge.js";
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

export function useWikiPageController(stream: Stream, slug: string, onClosed: () => void): WikiPageController {
  const [summary, setSummary] = useState<WikiPageSummary | null>(null);
  const [body, setBody] = useState<string>("");
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState<string>("");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [notFound, setNotFound] = useState(false);
  const [draftInitialized, setDraftInitialized] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const page = await readWikiPage(slug);
      setSummary(page?.summary ?? null);
      setBody(page?.body ?? "");
      setNotFound(page === null);
      setLoadError(null);
    } catch (error) {
      setLoadError(String(error));
      setNotFound(false);
    }
  }, [slug]);

  useEffect(() => {
    void refresh();
    setEditing(false);
  }, [refresh]);

  // Stable subscription — see WikiPageTab original for the rationale.
  const refreshRef = useRef(refresh);
  useEffect(() => { refreshRef.current = refresh; }, [refresh]);
  useEffect(() => subscribeOxplowEvents((event) => {
    if (knowledgeChanged(event as Record<string, unknown>)) void refreshRef.current();
  }), [slug]);

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
  }, [stream.id, slug, draft]);

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
  }, [stream.id, slug]);

  const remove = useCallback(async () => {
    if (!window.confirm(`Delete wiki page "${slug}"? The file will be removed.`)) return;
    try {
      await deleteWikiPage(slug, true);
      onClosed();
    } catch (error) {
      recordOpError({
        label: `Delete wiki page "${slug}"`,
        message: String(error),
      });
    }
  }, [stream.id, slug, onClosed]);

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
