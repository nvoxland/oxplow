import { createContext, useContext, useRef, useState } from "react";
import { runCommand } from "../api.js";
import { useAppPreference } from "../prefs/remembered.js";
import { pageUri, wikiUri } from "../uri/uri.js";
import { pageSlug, titledPage, viewWikiBody } from "./viewActions.js";

type SaveCommand = (name: string, input: unknown) => Promise<unknown>;
export const SaveViewCommandContext = createContext<SaveCommand | null>(null);

/** A shown view becomes a normal wiki page or an extension page. */
export function SaveViewForm({ kind, specYaml, initialTitle, onClose, onSaved }: {
  kind: "wiki" | "page";
  specYaml: string;
  initialTitle: string;
  onClose(): void;
  onSaved(uri: string): void;
}) {
  const command = useContext(SaveViewCommandContext) ?? runCommand;
  const [title, setTitle] = useState(initialTitle);
  const [preferred, rememberExtension] = useAppPreference("pages.extension", "my-pages");
  const [extension, setExtension] = useState(preferred);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const saving = useRef(false);
  const slug = pageSlug(title);
  const save = async () => {
    if (saving.current || !slug || (kind === "page" && !extension.trim())) return;
    saving.current = true;
    setBusy(true);
    setError(null);
    try {
      const yaml = titledPage(specYaml, title.trim());
      if (kind === "wiki") {
        await command("oxplow.knowledge.write_page", { slug, title: title.trim(), body: viewWikiBody(title.trim(), yaml) });
        onSaved(wikiUri(slug));
      } else {
        const name = extension.trim();
        await command("oxplow.extension.write_page", { extension: name, page: slug, spec: yaml, deploy: true });
        rememberExtension(name);
        onSaved(pageUri(`${name}/${slug}`));
      }
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      saving.current = false;
      setBusy(false);
    }
  };
  return <form className="ox-form" data-testid="save-view-form" onSubmit={(e) => { e.preventDefault(); void save(); }} onKeyDown={(e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onClose(); }
  }}>
    <label>Title<input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} data-testid="save-view-title" /></label>
    {kind === "page" ? <label>Extension<input value={extension} onChange={(e) => setExtension(e.target.value)} data-testid="save-view-extension" /></label> : null}
    {error ? <div className="ox-error" role="alert">{error}</div> : null}
    <button type="submit" disabled={busy || !slug || (kind === "page" && !extension.trim())} data-testid="save-view-submit">{busy ? "Saving…" : "Save"}</button>
    <button type="button" onClick={onClose}>Cancel</button>
  </form>;
}
