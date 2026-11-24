/// An extension's `links:` in markdown (`linkRules.ts`): a rehype plugin
/// that makes each match in prose a link to its URI — never inside code
/// or a link it is already in. The link is then any markdown link: a click
/// follows it, a right-click has Open In ▸ and its commands (`links.ts`).
import type { Element, ElementContent, Root, RootContent, Text } from "hast";

import { findRuleLinks } from "../../linkRules.js";
import type { LinkRuleDecl } from "../../tauri-bridge/generated/bindings.js";

const SKIP = new Set(["a", "code", "pre", "script", "style"]);

/** The plugin, over `rules` (`rulesOf(useExtensions())`). */
export function rehypeRuleLinks(rules: readonly LinkRuleDecl[]) {
  return (tree: Root) => {
    if (rules.length === 0) return;
    walk(tree, rules);
  };
}

function walk(node: Root | Element, rules: readonly LinkRuleDecl[]): void {
  const next: (RootContent | ElementContent)[] = [];
  for (const child of node.children) {
    if (child.type === "text") {
      next.push(...split(child, rules));
      continue;
    }
    if (child.type === "element" && !SKIP.has(child.tagName)) walk(child, rules);
    next.push(child);
  }
  node.children = next as typeof node.children;
}

function split(text: Text, rules: readonly LinkRuleDecl[]): (Text | Element)[] {
  const links = findRuleLinks(text.value, rules);
  if (links.length === 0) return [text];
  const out: (Text | Element)[] = [];
  let at = 0;
  for (const l of links) {
    if (l.start > at) out.push({ type: "text", value: text.value.slice(at, l.start) });
    out.push({ type: "element", tagName: "a", properties: { href: l.uri }, children: [{ type: "text", value: l.text }] });
    at = l.end;
  }
  if (at < text.value.length) out.push({ type: "text", value: text.value.slice(at) });
  return out;
}
