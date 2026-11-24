import { expect, test } from "bun:test";
import type { Element, Root, RootContent } from "hast";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { unified } from "unified";

import { rehypeRuleLinks } from "./ruleLinks.js";

// An extension's `links:` make text in markdown a link, outside code and
// outside a link it is already in.
const rules = [{ id: "issue", pattern: "[A-Z]{2,10}-[0-9]+", uri: "https://linear.app/acme/issue/{{match}}" }];
const parser = unified().use(remarkParse).use(remarkGfm).use(remarkRehype);
// Enough HTML to read the tree by.
const out = (n: RootContent | Root): string => {
  if (n.type === "text") return n.value;
  if (n.type === "element") {
    const e = n as Element;
    const href = e.properties?.href ? ` href="${String(e.properties.href)}"` : "";
    return `<${e.tagName}${href}>${e.children.map(out).join("")}</${e.tagName}>`;
  }
  if (n.type === "root") return n.children.map(out).join("");
  return "";
};
const html = (md: string) => {
  const tree = parser.runSync(parser.parse(md)) as Root;
  rehypeRuleLinks(rules)(tree);
  return out(tree).replace(/\n/g, "");
};

test("a match in prose is a link to its URI", () => {
  expect(html("Fixes PROJ-12 today.")).toBe('<p>Fixes <a href="https://linear.app/acme/issue/PROJ-12">PROJ-12</a> today.</p>');
});

test("code and links are left as they are", () => {
  expect(html("`PROJ-12` and [see PROJ-3](https://x.io)")).toBe('<p><code>PROJ-12</code> and <a href="https://x.io">see PROJ-3</a></p>');
  expect(html("```\nPROJ-12\n```")).toBe("<pre><code>PROJ-12</code></pre>");
});

test("no rules, no change", () => {
  const tree: Root = { type: "root", children: [{ type: "text", value: "PROJ-12" }] };
  rehypeRuleLinks([])(tree);
  expect(tree.children).toEqual([{ type: "text", value: "PROJ-12" }]);
});
