/// An extension's `links:` (`link_rules.rs`): text that is a link — an
/// issue key, a ticket number — made one in the terminal and in markdown.
/// The same rule as the daemon's `find_rule_links`: whole-word matches,
/// leftmost first, the earlier rule's where two overlap, the match
/// percent-encoded into the URI. `link_rules_cases.json` holds the cases
/// both run.
import type { Extension, LinkRuleDecl } from "./tauri-bridge/generated/bindings.js";

export interface RuleLink {
  start: number;
  end: number;
  text: string;
  uri: string;
}

/** The rules of the enabled extensions, in their order. */
export function rulesOf(extensions: Extension[] | null): LinkRuleDecl[] {
  return (extensions ?? []).filter((e) => e.enabled).flatMap((e) => e.links ?? []);
}

/** Percent-encode `text` for a URI: every byte but `A-Z a-z 0-9 - . _ ~`. */
export function encodeMatch(text: string): string {
  return Array.from(new TextEncoder().encode(text), (b) => {
    const c = String.fromCharCode(b);
    return /[A-Za-z0-9\-._~]/.test(c) ? c : `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
  }).join("");
}

const compiled = new Map<string, RegExp | null>();
function regexOf(pattern: string): RegExp | null {
  if (!compiled.has(pattern)) {
    let re: RegExp | null = null;
    try {
      re = new RegExp(pattern, "g");
    } catch {
      re = null;
    }
    compiled.set(pattern, re);
  }
  return compiled.get(pattern) ?? null;
}

const word = (c: string | undefined) => c !== undefined && /[A-Za-z0-9_]/.test(c);

/** The links `rules` make in `text`. */
export function findRuleLinks(text: string, rules: readonly LinkRuleDecl[]): RuleLink[] {
  const found: { order: number; link: RuleLink }[] = [];
  rules.forEach((rule, order) => {
    const re = regexOf(rule.pattern);
    if (!re) return;
    re.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(text)) !== null) {
      const start = m.index;
      const end = start + m[0].length;
      if (m[0].length === 0) {
        re.lastIndex += 1;
        continue;
      }
      if (word(text[start - 1]) || word(text[end])) continue;
      found.push({ order, link: { start, end, text: m[0], uri: rule.uri.replace("{{match}}", encodeMatch(m[0])) } });
    }
  });
  found.sort((a, b) => a.order - b.order || a.link.start - b.link.start);
  const kept: RuleLink[] = [];
  for (const { link } of found) {
    if (kept.every((k) => link.end <= k.start || link.start >= k.end)) kept.push(link);
  }
  return kept.sort((a, b) => a.start - b.start);
}
