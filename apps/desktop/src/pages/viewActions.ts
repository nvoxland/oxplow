import { parseDocument } from "yaml";
import { withUriView } from "../uri/uri.js";
import { RENDERER_VIEW_KEY } from "./renderers.js";
import type { SqlCell } from "../tauri-bridge/generated/bindings.js";

/** Keep the YAML as a document: a title edit preserves comments and key order. */
export function titledPage(specYaml: string, title: string): string {
  const doc = parseDocument(specYaml);
  if (doc.errors.length) throw new Error(doc.errors[0].message);
  doc.set("title", title);
  return doc.toString();
}

export function viewWikiBody(title: string, specYaml: string): string {
  // A source may contain markdown fences: use a longer enclosing fence.
  const ticks = Math.max(3, ...(specYaml.match(/`+/g) ?? []).map((s) => s.length + 1));
  const fence = "`".repeat(ticks);
  return `# ${title.replace(/[\r\n]+/g, " ")}\n\n${fence}oxplow-page\n${specYaml.trimEnd()}\n${fence}\n`;
}

/** The extension a shown view's page is under: `temp/<id>`, the page file
 *  `<id>.yaml` on `oxplow:temp`. */
export const TEMP_EXTENSION = "temp";

/** The page "Show Definition" draws with: Definition, a display mode of a
 *  page's YAML file. */
export const DEFINITION_PAGE = "oxplow-foundation/definition";
/** The page that shows a file raw, read-only, at any revision. */
export const FILE_VIEWER_PAGE = "oxplow-foundation/file-viewer";

/** What "Show Definition" opens: the page's running copy
 *  (`PageProvenance.running_uri`, its file on `oxplow:extensions` pinned to
 *  the deployment) drawn as Definition, with the run's exact params as the
 *  mode's `params` filter. */
export function definitionUri(runningUri: string, params: Record<string, SqlCell>): string {
  return withUriView(runningUri, { [RENDERER_VIEW_KEY]: DEFINITION_PAGE, params: JSON.stringify(params) });
}

export function pageSlug(title: string): string {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return /^\d/.test(slug) ? `page-${slug}` : slug;
}
