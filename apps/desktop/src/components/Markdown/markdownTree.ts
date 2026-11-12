/**
 * Markdown as a tree a component's frame may draw (the kit's
 * `renderMarkdown`): the viewer's parse (remark + GFM), its wikilinks, link
 * rules and titles, reduced to the elements a frame draws — no HTML crosses
 * the bridge, and the library builds each node with `createElement`.
 *
 * A link is a ref oxplow opens (`ref`), a web page (`href`, http or https
 * only) or broken (`broken`, the reason). One line (`inline`) keeps only
 * `INLINE_ELEMENTS`; a whole body adds `BLOCK_ELEMENTS`. Anything else —
 * raw HTML, an image, a table — unwraps into its text, or drops when it has
 * none.
 *
 * It imports no React (the test harness builds it, `harnessBuild.test.ts`):
 * its lookups come in as `TreeDeps` — the app's title caches
 * (`liveTreeDeps`), or none in the harness.
 */
import type { Element as HastElement, ElementContent, Root as HastRoot } from "hast";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";

import { INLINE_ELEMENTS } from "./inlineMarkdown.js";
import { linkRef, parseMarkdownLink, preprocessWikilinks, refLinkNames, urlTransform, type RefLinkNames } from "./markdownLinks.js";

/** The block elements a whole body keeps, beside the inline ones. */
export const BLOCK_ELEMENTS: readonly string[] = ["p", "ul", "ol", "li", "blockquote", "pre", "h1", "h2", "h3", "h4", "h5", "h6", "hr"];

export type MarkdownNode = string | MarkdownElement;

export interface MarkdownElement {
  tag: string;
  children: MarkdownNode[];
  /** A link oxplow opens: the ref (`navigate`). */
  ref?: string;
  /** A link to a web page. */
  href?: string;
  /** A link that goes nowhere, and why. */
  broken?: string;
}

/** What the tree reads beyond the text: the active list's bare ids, and a
 *  ref link's title by the viewer's rule. */
export interface TreeDeps {
  mentionRef(id: string): string | null;
  refTitle(names: RefLinkNames, text: string): Promise<{ title: string | null; brokenReason: string | null }>;
}

const parser = unified().use(remarkParse).use(remarkGfm).use(remarkRehype);

/** `text` as the tree a frame draws: one line when `inline`. */
export async function markdownTree(text: string, inline: boolean, deps: TreeDeps): Promise<MarkdownNode[]> {
  const hast = (await parser.run(parser.parse(preprocessWikilinks(text, deps.mentionRef)))) as HastRoot;
  const allowed = new Set(inline ? INLINE_ELEMENTS : [...INLINE_ELEMENTS, ...BLOCK_ELEMENTS]);
  const nodes = merge(await convertAll(hast.children as ElementContent[], allowed, deps), inline);
  return inline ? trimLine(nodes) : nodes.filter((n) => typeof n !== "string" || n.trim() !== "");
}

async function convertAll(children: ElementContent[], allowed: ReadonlySet<string>, deps: TreeDeps): Promise<MarkdownNode[]> {
  return (await Promise.all(children.map((c) => convert(c, allowed, deps)))).flat();
}

async function convert(node: ElementContent, allowed: ReadonlySet<string>, deps: TreeDeps): Promise<MarkdownNode[]> {
  if (node.type === "text") return [node.value];
  if (node.type !== "element") return [];
  const children = merge(await convertAll(node.children, allowed, deps), false);
  if (node.tagName === "a") return [await link(node, children, deps)];
  if (!allowed.has(node.tagName)) return children;
  // Between blocks, the parse's newlines are only layout.
  const kept = BLOCK_ELEMENTS.includes(node.tagName) && node.tagName !== "pre" ? children.filter((c) => typeof c !== "string" || c.trim() !== "") : children;
  return [{ tag: node.tagName, children: kept }];
}

async function link(node: HastElement, children: MarkdownNode[], deps: TreeDeps): Promise<MarkdownNode> {
  const href = urlTransform(typeof node.properties?.href === "string" ? node.properties.href : "");
  if (!href) return { tag: "span", children };
  const parsed = parseMarkdownLink(href);
  if (parsed.kind === "broken") return { tag: "a", broken: parsed.reason, children };
  const ref = linkRef(parsed);
  if (ref) {
    const text = children.length === 1 && typeof children[0] === "string" ? children[0] : null;
    const { title, brokenReason } = await deps.refTitle(refLinkNames(parsed), text ?? "");
    if (brokenReason) return { tag: "a", broken: brokenReason, children };
    return { tag: "a", ref, children: title ? [title] : children };
  }
  if (/^https?:\/\//i.test(href)) return { tag: "a", href, children };
  // A script, an anchor or anything else a frame can't follow: its text.
  return { tag: "span", children };
}

/** Adjacent strings as one; a line's unwrapped `span`s and newlines into
 *  its text. */
function merge(nodes: MarkdownNode[], line: boolean): MarkdownNode[] {
  const out: MarkdownNode[] = [];
  for (const n of nodes.flatMap((n) => (typeof n !== "string" && n.tag === "span" ? n.children : [n]))) {
    const s = typeof n === "string" && line ? n.replace(/\s+/g, " ") : n;
    const last = out[out.length - 1];
    if (typeof s === "string" && typeof last === "string") out[out.length - 1] = line ? (last + s).replace(/\s+/g, " ") : last + s;
    else out.push(s);
  }
  return out;
}

function trimLine(nodes: MarkdownNode[]): MarkdownNode[] {
  const out = [...nodes];
  if (typeof out[0] === "string") out[0] = out[0].trimStart();
  const end = out.length - 1;
  if (typeof out[end] === "string") out[end] = (out[end] as string).trimEnd();
  return out.filter((n) => n !== "");
}

/** No lookups: bare ids name no item, and no link gets a title (the test
 *  harness's host, which reads nothing of the app's). */
export const NO_LOOKUPS: TreeDeps = {
  mentionRef: () => null,
  refTitle: async () => ({ title: null, brokenReason: null }),
};
