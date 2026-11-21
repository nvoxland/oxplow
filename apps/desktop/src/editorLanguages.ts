import { onRemoteReconnect, querySql, subscribeOxplowEvents } from "./api.js";
import { setDeclaredEditorLanguages } from "./editor-language.js";
import { extensionsChanged } from "./viz/rerun.js";

/**
 * The languages the enabled extensions declare for the editor
 * (`v_language.monaco`, from `editor: { monaco }`), kept in
 * `editor-language.ts` so `languageForPath` names a third-party language's
 * files without a change to the app. Loaded once Monaco is, and again when
 * the extensions' definitions change or the daemon reconnects.
 */
let started = false;

async function load(): Promise<void> {
  const res = await querySql(
    "SELECT extensions, monaco FROM v_language WHERE enabled = 1 AND monaco IS NOT NULL ORDER BY extension, id",
    [],
    1000,
  );
  setDeclaredEditorLanguages(
    res.rows.map((r) => ({
      extensions: JSON.parse(String(r[0])) as string[],
      monaco: String(r[1]),
    })),
  );
}

export function startEditorLanguages(): Promise<void> {
  if (!started) {
    started = true;
    const reload = () => {
      load().catch(() => {
        /* the table of well-known extensions still draws every file */
      });
    };
    subscribeOxplowEvents((event) => {
      if (extensionsChanged(event as Record<string, unknown>)) reload();
    });
    onRemoteReconnect(reload);
  }
  return load().catch(() => {});
}
