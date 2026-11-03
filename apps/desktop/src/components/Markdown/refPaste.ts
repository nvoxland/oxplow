/**
 * What a paste into the markdown editor is (`.context/markdown.md` "Paste"):
 * a pasted oxplow URI is a ref the editor puts in as a link (`refLinkOf`),
 * a pasted web URL a link for the selection. Pure.
 */
import { parseUri } from "../../uri/uri.js";

/** A ref to put in: its URI and what it says when it has no bare form. */
export interface RefLink {
  uri: string;
  title: string;
}

/** What pasted text is when it is only a link: an oxplow URI (a ref) or a
 *  web URL; null for any other text. */
export function pastedLink(text: string): { kind: "ref"; uri: string } | { kind: "url"; url: string } | null {
  const t = text.trim();
  if (!t || /\s/.test(t)) return null;
  if (t.startsWith("oxplow:")) return parseUri(t) ? { kind: "ref", uri: t } : null;
  return /^https?:\/\/\S+$/i.test(t) ? { kind: "url", url: t } : null;
}
