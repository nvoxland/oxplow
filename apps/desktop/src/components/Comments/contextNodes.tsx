/// Context nodes: how a page region declares *what it is* so a text
/// selection can discover the typed entity (and the hierarchy of
/// entities) it sits inside.
///
/// Any element that represents a canonical thing carries two data
/// attributes using the same `(kind,id)` vocabulary as tab ids and the
/// `page_ref` graph (`file` / `directory` / `wiki` / `task` /
/// `commit` / `finding`, extensible):
///
///   <div data-ref-kind="commit" data-ref-id="abc1234"> … </div>
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

/// A canonical cross-page reference — matches the backend
/// `CommentTarget` `{ kind, id }` shape (and the `page_ref` vocabulary).
export interface RefNode {
  kind: string;
  id: string;
}

const KIND_ATTR = "data-ref-kind";
const ID_ATTR = "data-ref-id";

/// The nearest element at or above `node` that carries BOTH `data-ref-*`
/// attributes, read as a [`RefNode`]. A node with only one of the two
/// attributes is malformed and skipped (we keep climbing).
function refOf(el: Element): RefNode | null {
  const kind = el.getAttribute(KIND_ATTR);
  const id = el.getAttribute(ID_ATTR);
  if (kind && id) return { kind, id };
  return null;
}

/// Resolve the starting element for an ancestor walk. Selections often
/// anchor on a text node, which has no attributes of its own — climb to
/// its parent element.
function startElement(node: Node | null): Element | null {
  if (!node) return null;
  return node instanceof Element ? node : node.parentElement;
}

/// Walk from `node` up through its ancestors collecting every context
/// node, innermost→outermost. Adjacent duplicates (the same `(kind,id)`
/// declared on a wrapper and its child) collapse to one entry, so a
/// region can be marked on multiple nested elements without polluting
/// the chain.
export function collectContextChain(node: Node | null): RefNode[] {
  const chain: RefNode[] = [];
  let el = startElement(node);
  let lastKey = "";
  while (el) {
    const ref = refOf(el);
    if (ref) {
      const key = `${ref.kind}\u0000${ref.id}`;
      if (key !== lastKey) {
        chain.push(ref);
        lastKey = key;
      }
    }
    el = el.parentElement;
  }
  return chain;
}

/// The innermost context node at or above `node`, or `null` when the
/// selection sits in a region that declares no typed identity (the
/// caller then falls back to the active page's own tab ref).
export function nearestContextNode(node: Node | null): RefNode | null {
  return collectContextChain(node)[0] ?? null;
}

/// The nearest ELEMENT at or above `node` that carries both `data-ref-*`
/// attributes — the anchoring element whose `textContent` a plain-DOM
/// comment's quote is resolved within. Returns `null` when the selection
/// sits outside any context node.
export function nearestContextElement(node: Node | null): Element | null {
  const start = startElement(node);
  return start ? start.closest(`[${KIND_ATTR}][${ID_ATTR}]`) : null;
}

/// Read the [`RefNode`] declared directly on `el`, or `null` when `el`
/// is missing either attribute.
export function refOfElement(el: Element): RefNode | null {
  return refOf(el);
}

/// The data attributes that mark an element as a context node. Spread
/// onto any element to declare its canonical identity:
///
///   <tr {...contextNodeProps("work_item", `oxplow:${id}`)}> … </tr>
export function contextNodeProps(kind: string, id: string): Record<string, string> {
  return { [KIND_ATTR]: kind, [ID_ATTR]: id };
}

/// Hook form of [`contextNodeProps`] for call sites that prefer it.
export function useContextNode(kind: string, id: string): Record<string, string> {
  return contextNodeProps(kind, id);
}

/// Declarative wrapper: renders a `<div>` (or the given `as` element)
/// tagged as a context node. Use when there isn't already an element to
/// hang the attributes on.
export function ContextNode({
  kind,
  id,
  children,
  className,
}: {
  kind: string;
  id: string;
  children?: ReactNode;
  className?: string;
}): ReactNode {
  return (
    <div className={className} {...contextNodeProps(kind, id)}>
      {children}
    </div>
  );
}
