import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ContextMenu } from "../ContextMenu.js";
import type { MenuItem, MenuPosition } from "../../menu.js";

// Mermaid is loaded lazily so this module is safe to import in
// non-DOM test environments (parseMarkdownLink is the main reason
// to import without mounting the component).
let mermaidPromise: Promise<typeof import("mermaid").default> | null = null;
function loadMermaid() {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((mod) => {
      mod.default.initialize({ startOnLoad: false, theme: "dark", securityLevel: "loose" });
      return mod.default;
    });
  }
  return mermaidPromise;
}

export type ParsedLink =
  | { kind: "empty" }
  | { kind: "anchor" }
  | { kind: "external" }
  | { kind: "internal"; slug: string };

/**
 * Classify a markdown link href. Shared by NoteTab (wiki navigation) and
 * WorkItemDetail (work-item modal markdown rendering). Pure — easy to test.
 */
export function parseMarkdownLink(rawHref: string): ParsedLink {
  if (!rawHref) return { kind: "empty" };
  if (rawHref.startsWith("#")) return { kind: "anchor" };
  if (/^https?:\/\//i.test(rawHref) || rawHref.startsWith("mailto:")) {
    return { kind: "external" };
  }
  let target = rawHref.replace(/^\.?\//, "");
  target = target.split("#")[0]?.split("?")[0] ?? "";
  if (target.endsWith(".md")) target = target.slice(0, -3);
  return target ? { kind: "internal", slug: target } : { kind: "empty" };
}

export interface MarkdownViewProps {
  body: string;
  /** Optional internal link handler (NoteTab routes to wiki history). */
  onNavigateInternal?: (slug: string) => void;
  /** Optional new-tab handler (NoteTab opens slug in another tab). */
  onOpenInNewTab?: (slug: string) => void;
  /**
   * Render mermaid code blocks as SVG diagrams. NoteTab uses this; the
   * work-item modal disables it (default false) since work-item notes
   * tend to be short and a stray code fence shouldn't trigger rendering.
   */
  renderMermaid?: boolean;
  /** Apply max-height + internal scroll instead of growing unbounded. */
  maxHeight?: number | string;
  /** Extra style overrides for the outer wrapper. */
  style?: CSSProperties;
  className?: string;
}

/**
 * Generic safe-markdown renderer used by Notes (wiki) and the
 * Plan work-item modal. Sanitization is delegated to react-markdown +
 * remark-gfm, which strip raw HTML by default — no scripts, no
 * arbitrary external fetches beyond standard markdown links/images.
 *
 * Link behavior:
 * - external links open in the OS browser (Electron `_blank`).
 * - anchor (`#…`) links use default behavior (in-page jump).
 * - internal links route through `onNavigateInternal` / `onOpenInNewTab`
 *   when those handlers are supplied; otherwise they no-op (work-item
 *   notes don't have a wiki to navigate to).
 */
export function MarkdownView({
  body,
  onNavigateInternal,
  onOpenInNewTab,
  renderMermaid = false,
  maxHeight,
  style,
  className,
}: MarkdownViewProps) {
  const ref = useRef<HTMLDivElement | null>(null);
  const [linkMenu, setLinkMenu] = useState<{
    position: MenuPosition;
    href: string;
    kind: "internal" | "external";
    internalSlug?: string;
  } | null>(null);

  const handleLinkClick = useCallback((event: React.MouseEvent<HTMLAnchorElement>) => {
    const href = event.currentTarget.getAttribute("href") ?? "";
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "anchor") return;
    event.preventDefault();
    if (parsed.kind === "empty") return;
    if (parsed.kind === "external") {
      window.open(href, "_blank", "noopener,noreferrer");
      return;
    }
    // Internal link
    const newTab = event.metaKey || event.ctrlKey || event.button === 1;
    if (newTab && onOpenInNewTab) onOpenInNewTab(parsed.slug);
    else if (onNavigateInternal) onNavigateInternal(parsed.slug);
    // No handlers? Silently ignore — work-item notes don't have wiki nav.
  }, [onNavigateInternal, onOpenInNewTab]);

  const handleLinkContextMenu = useCallback((event: React.MouseEvent<HTMLAnchorElement>) => {
    const href = event.currentTarget.getAttribute("href") ?? "";
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "anchor" || parsed.kind === "empty") return;
    event.preventDefault();
    setLinkMenu({
      position: { x: event.clientX, y: event.clientY },
      href,
      kind: parsed.kind === "external" ? "external" : "internal",
      internalSlug: parsed.kind === "internal" ? parsed.slug : undefined,
    });
  }, []);

  // Mermaid rendering pass — opt-in via renderMermaid flag. Replaces
  // <pre><code class="language-mermaid">…</code></pre> blocks with SVG.
  useEffect(() => {
    if (!renderMermaid) return;
    const root = ref.current;
    if (!root) return;
    const blocks = root.querySelectorAll<HTMLElement>("code.language-mermaid");
    blocks.forEach(async (code, idx) => {
      const source = code.textContent ?? "";
      const id = `mermaid-${Date.now()}-${idx}`;
      try {
        const mermaid = await loadMermaid();
        const { svg } = await mermaid.render(id, source);
        const host = document.createElement("div");
        host.className = "mermaid-rendered";
        host.innerHTML = svg;
        const pre = code.parentElement;
        if (pre && pre.tagName === "PRE") pre.replaceWith(host);
      } catch (error) {
        const pre = code.parentElement;
        if (pre && pre.tagName === "PRE") {
          const err = document.createElement("div");
          err.style.color = "var(--severity-critical)";
          err.style.fontSize = "12px";
          err.textContent = `Mermaid parse error: ${String(error)}`;
          pre.after(err);
        }
      }
    });
  }, [body, renderMermaid]);

  const menuItems: MenuItem[] = useMemo(() => {
    if (!linkMenu) return [];
    if (linkMenu.kind === "internal" && linkMenu.internalSlug) {
      const target = linkMenu.internalSlug;
      const items: MenuItem[] = [];
      if (onNavigateInternal) {
        items.push({ id: "open", label: "Open", enabled: true, run: () => onNavigateInternal(target) });
      }
      if (onOpenInNewTab) {
        items.push({ id: "open-new", label: "Open in new tab", enabled: true, run: () => onOpenInNewTab(target) });
      }
      return items;
    }
    return [
      { id: "open-ext", label: "Open in browser", enabled: true, run: () => { window.open(linkMenu.href, "_blank", "noopener,noreferrer"); } },
      { id: "copy", label: "Copy link", enabled: true, run: () => { void navigator.clipboard.writeText(linkMenu.href).catch(() => {}); } },
    ];
  }, [linkMenu, onNavigateInternal, onOpenInNewTab]);

  const wrapperStyle: CSSProperties = {
    ...(maxHeight !== undefined ? { maxHeight, overflowY: "auto" } : {}),
    ...style,
  };

  return (
    <div ref={ref} className={className} style={wrapperStyle}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ node, ...props }) => (
            <a
              {...props}
              onClick={handleLinkClick}
              onAuxClick={handleLinkClick}
              onContextMenu={handleLinkContextMenu}
            />
          ),
        }}
      >
        {body}
      </ReactMarkdown>
      {linkMenu && (
        <ContextMenu
          items={menuItems}
          position={linkMenu.position}
          onClose={() => setLinkMenu(null)}
        />
      )}
    </div>
  );
}
