/// Code blocks' syntax highlighting, the viewer's and the editor's: one
/// lowlight (highlight.js grammars) that both use, keyed by the fence's
/// language (```` ```sql ````). A fence with no language, or one it
/// doesn't know, stays plain. The colors are the theme's `--syntax-*`.
import type { Element, Root, Text } from "hast";
import { common, createLowlight } from "lowlight";

export const lowlight = createLowlight(common);

/** A rehype plugin for the viewer: each `<pre><code class="language-x">`
 *  of a language lowlight knows gets its highlighted spans. */
export function rehypeHighlightCode() {
  return (tree: Root) => {
    visit(tree, (pre) => {
      const code = pre.children[0];
      if (pre.tagName !== "pre" || code?.type !== "element" || code.tagName !== "code") return;
      const lang = languageOf(code);
      if (!lang || !lowlight.registered(lang)) return;
      const text = code.children.map((c) => (c.type === "text" ? (c as Text).value : "")).join("");
      code.children = lowlight.highlight(lang, text).children as Element["children"];
    });
  };
}

function languageOf(code: Element): string | null {
  const classes = code.properties?.className;
  const list = Array.isArray(classes) ? classes : [];
  const lang = list.map(String).find((c) => c.startsWith("language-"));
  return lang ? lang.slice("language-".length) : null;
}

function visit(node: Root | Element, fn: (el: Element) => void) {
  for (const child of node.children) {
    if (child.type !== "element") continue;
    fn(child);
    visit(child, fn);
  }
}
