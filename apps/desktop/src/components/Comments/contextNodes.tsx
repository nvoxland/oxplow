/// Context nodes: how a page region declares *what it is* so a text
/// selection can discover the typed entity (and the hierarchy of
/// entities) it sits inside.
///
/// Any element that represents a canonical thing carries one data
/// attribute: its URI, the same one tab ids and the `page_ref` graph use:
///
///   <div data-ref="oxplow:oxplow/commit?sha=abc1234"> … </div>
///
/// Because `window.getSelection()` hands back raw DOM nodes, the only
/// uniform way to ask "what typed region is this selection in?" across
/// arbitrary pages (task rows, file lists, the git graph, section
/// headers) is to walk DOM ancestors. Nesting in the DOM IS the
/// hierarchy: a file row inside a commit card inside the git dashboard
/// yields the chain `[file, commit, git-dashboard]`, innermost
/// first. The innermost node is the comment's primary target; the rest
/// is its context chain.

import type { ReactNode } from "react";

const REF_ATTR = "data-ref";

/// The URI an element declares, or null when it declares none.
function refOf(el: Element): string | null {
  return el.getAttribute(REF_ATTR) || null;
}

/// Resolve the starting element for an ancestor walk. Selections often
/// anchor on a text node, which has no attributes of its own — climb to
/// its parent element.
function startElement(node: Node | null): Element | null {
  if (!node) return null;
  return node instanceof Element ? node : node.parentElement;
}

/// Walk from `node` up through its ancestors collecting every context
/// node, innermost→outermost. Adjacent duplicates (the same URI
/// declared on a wrapper and its child) collapse to one entry, so a
/// region can be marked on multiple nested elements without polluting
/// the chain.
export function collectContextChain(node: Node | null): string[] {
  const chain: string[] = [];
  let el = startElement(node);
  let last = "";
  while (el) {
    const ref = refOf(el);
    if (ref && ref !== last) {
      chain.push(ref);
      last = ref;
    }
    el = el.parentElement;
  }
  return chain;
}

/// The innermost context node at or above `node`, or `null` when the
/// selection sits in a region that declares no typed identity (the
/// caller then falls back to the active page's own tab ref).
export function nearestContextNode(node: Node | null): string | null {
  return collectContextChain(node)[0] ?? null;
}

/// The nearest ELEMENT at or above `node` that carries a `data-ref` — the anchoring element whose `textContent` a plain-DOM
/// comment's quote is resolved within. Returns `null` when the selection
/// sits outside any context node.
export function nearestContextElement(node: Node | null): Element | null {
  const start = startElement(node);
  return start ? start.closest(`[${REF_ATTR}]`) : null;
}

/// Read the URI declared directly on `el`, or `null` when it declares none.
export function refOfElement(el: Element): string | null {
  return refOf(el);
}

/// The data attribute that marks an element as a context node. Spread
/// onto any element to declare its canonical identity:
///
///   <tr {...contextNodeProps(item.ref)}> … </tr>
export function contextNodeProps(uri: string): Record<string, string> {
  return { [REF_ATTR]: uri };
}

/// Hook form of [`contextNodeProps`] for call sites that prefer it.
export function useContextNode(uri: string): Record<string, string> {
  return contextNodeProps(uri);
}

/// Declarative wrapper: renders a `<div>` tagged as a context node. Use
/// when there isn't already an element to hang the attribute on.
export function ContextNode({
  uri,
  children,
  className,
}: {
  uri: string;
  children?: ReactNode;
  className?: string;
}): ReactNode {
  return (
    <div className={className} {...contextNodeProps(uri)}>
      {children}
    </div>
  );
}
