import { parseDocument } from "yaml";
import { pageUri } from "../uri/uri.js";
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

export function definitionUri(page: string, params: Record<string, SqlCell>): string {
  return pageUri("definition", { of: page, params: JSON.stringify(params) });
}

export function pageSlug(title: string): string {
  const slug = title.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
  return /^\d/.test(slug) ? `page-${slug}` : slug;
}
