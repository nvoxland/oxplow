/// What the Context panel calls a collected item: the label it was added
/// with, else its page's title (the one title cache, `useRefTitle`: an
/// agent session's title, a task's), else a file's path and lines, else the
/// start of its text. Never a raw `oxplow:` URI when a name can be had.
import { useRefTitle } from "../../refTitleCache.js";
import { isOxplowScheme, paramOf, typeOf, viewOf } from "../../uri/uri.js";

export interface Titled {
  label?: string | null;
  uri?: string | null;
  text?: string | null;
}

/** A name for `uri` from the URI alone: a file's path (and its lines), a
 *  page's id; else the URI. */
export function uriName(uri: string): string {
  if (!isOxplowScheme(uri)) return uri;
  const path = paramOf(uri, "path");
  if (path) {
    const lines = viewOf(uri, "lines");
    return lines ? `${path}:${lines}` : path;
  }
  return paramOf(uri, "page") ?? uri;
}

/** The name from what is known without reading: the label, else the URI's
 *  name, else the text's start. */
export function plainTitle(what: Titled): string {
  return what.label || (what.uri ? uriName(what.uri) : (what.text ?? "").slice(0, 80));
}

/** The title of what was read for its page, when there is one. */
export function useResolvedTitle(uri: string | null | undefined): string | null {
  const found = useRefTitle(uri && typeOf(uri) !== "oxplow/file" ? uri : null);
  return found.status === "found" && found.title ? found.title : null;
}

/** The name the panel shows for `what`. */
export function useContextTitle(what: Titled): string {
  const resolved = useResolvedTitle(what.label ? null : what.uri);
  return what.label || resolved || plainTitle(what);
}
