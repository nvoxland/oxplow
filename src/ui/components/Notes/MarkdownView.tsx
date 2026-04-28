import { useCallback, useEffect, useMemo, useRef } from "react";
import type { CSSProperties } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Kebab } from "../Kebab.js";
import type { MenuItem } from "../../menu.js";

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
  | { kind: "internal"; slug: string }
  | { kind: "file"; path: string; line?: number };

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
  if (rawHref.startsWith("file:")) {
    const raw = rawHref.slice("file:".length);
    if (!raw) return { kind: "empty" };
    const lineMatch = raw.match(/^(.+?):(\d+)$/);
    if (lineMatch) {
      return { kind: "file", path: lineMatch[1]!, line: Number(lineMatch[2]) };
    }
    return { kind: "file", path: raw };
  }
  let target = rawHref.replace(/^\.?\//, "");
  target = target.split("#")[0]?.split("?")[0] ?? "";
  if (target.endsWith(".md")) target = target.slice(0, -3);
  return target ? { kind: "internal", slug: target } : { kind: "empty" };
}

/**
 * Heuristic: does a wikilink target look like a repo file path rather
 * than a wiki note slug? File paths contain a slash or end in a recognizable
 * extension; bare slugs like `architecture` are routed to wiki navigation.
 */
function looksLikeFilePath(target: string): boolean {
  if (target.includes("/")) return true;
  // Tail extension other than .md → file. .md → wiki note.
  const dot = target.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = target.slice(dot + 1).toLowerCase();
  return ext.length > 0 && ext !== "md" && /^[a-z0-9]+$/i.test(ext);
}

/**
 * Preprocess `[[ ]]` wikilinks in a markdown body into standard markdown
 * links so the existing ReactMarkdown pipeline renders them clickable.
 *
 * Supported target shapes:
 * - `[[path/to/file.ts]]`         → file link
 * - `[[path/to/file.ts:42]]`      → file link with line
 * - `[[path/to/file.ts|label]]`   → file link with custom display text
 * - `[[some-slug]]`               → wiki internal link (note slug)
 *
 * Wikilinks inside fenced code blocks or inline code are left alone so
 * documentation about the syntax itself doesn't get rewritten.
 */
export function preprocessWikilinks(body: string): string {
  // Split out fenced code blocks (```...```) and protect them.
  const segments = body.split(/(```[\s\S]*?```)/g);
  return segments.map((seg, idx) => {
    if (idx % 2 === 1) return seg; // fenced block — leave alone
    return rewriteWikilinksOutsideInlineCode(seg);
  }).join("");
}

function rewriteWikilinksOutsideInlineCode(text: string): string {
  // Split on inline backtick spans. Even-index = prose, odd = code.
  const parts = text.split(/(`[^`\n]*`)/g);
  return parts.map((part, idx) => {
    if (idx % 2 === 1) return part;
    return part.replace(/\[\[([^\[\]\n|]+)(?:\|([^\[\]\n]+))?\]\]/g, (_match, rawTarget: string, label?: string) => {
      const target = rawTarget.trim();
      const display = (label ?? "").trim() || target;
      if (!target) return _match;
      if (looksLikeFilePath(target)) {
        return `[${display}](file:${target})`;
      }
      // Treat as wiki note slug.
      return `[${display}](${target})`;
    });
  }).join("");
}

export interface MarkdownViewProps {
  body: string;
  /** Optional internal link handler (NoteTab routes to wiki history). */
  onNavigateInternal?: (slug: string) => void;
  /** Optional new-tab handler (NoteTab opens slug in another tab). */
  onOpenInNewTab?: (slug: string) => void;
  /** Optional file-link handler — invoked for `[[path/to/file]]` wikilinks. */
  onOpenFile?: (path: string, line?: number) => void;
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
  onOpenFile,
  renderMermaid = false,
  maxHeight,
  style,
  className,
}: MarkdownViewProps) {
  const processedBody = useMemo(() => preprocessWikilinks(body), [body]);
  const ref = useRef<HTMLDivElement | null>(null);

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
    if (parsed.kind === "file") {
      onOpenFile?.(parsed.path, parsed.line);
      return;
    }
    // Internal link
    const newTab = event.metaKey || event.ctrlKey || event.button === 1;
    if (newTab && onOpenInNewTab) onOpenInNewTab(parsed.slug);
    else if (onNavigateInternal) onNavigateInternal(parsed.slug);
    // No handlers? Silently ignore — work-item notes don't have wiki nav.
  }, [onNavigateInternal, onOpenInNewTab, onOpenFile]);

  const buildLinkMenu = useCallback((href: string): MenuItem[] => {
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "internal") {
      const target = parsed.slug;
      const items: MenuItem[] = [];
      if (onNavigateInternal) {
        items.push({ id: "open", label: "Open", enabled: true, run: () => onNavigateInternal(target) });
      }
      if (onOpenInNewTab) {
        items.push({ id: "open-new", label: "Open in new tab", enabled: true, run: () => onOpenInNewTab(target) });
      }
      return items;
    }
    if (parsed.kind === "external") {
      return [
        { id: "open-ext", label: "Open in browser", enabled: true, run: () => { window.open(href, "_blank", "noopener,noreferrer"); } },
        { id: "copy", label: "Copy link", enabled: true, run: () => { void navigator.clipboard.writeText(href).catch(() => {}); } },
      ];
    }
    if (parsed.kind === "file") {
      const items: MenuItem[] = [];
      if (onOpenFile) {
        items.push({ id: "open-file", label: "Open file", enabled: true, run: () => onOpenFile(parsed.path, parsed.line) });
      }
      items.push({ id: "copy-path", label: "Copy path", enabled: true, run: () => { void navigator.clipboard.writeText(parsed.path).catch(() => {}); } });
      return items;
    }
    return [];
  }, [onNavigateInternal, onOpenInNewTab, onOpenFile]);

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

  const wrapperStyle: CSSProperties = {
    ...(maxHeight !== undefined ? { maxHeight, overflowY: "auto" } : {}),
    ...style,
  };

  return (
    <div ref={ref} className={className} style={wrapperStyle}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ node, ...props }) => {
            const href = (props.href as string | undefined) ?? "";
            const parsed = parseMarkdownLink(href);
            // Anchor and empty links don't get a kebab — there's no useful
            // action besides "jump in page" for those.
            if (parsed.kind === "anchor" || parsed.kind === "empty") {
              return <a {...props} onClick={handleLinkClick} onAuxClick={handleLinkClick} />;
            }
            const items = buildLinkMenu(href);
            return (
              <span style={{ display: "inline-flex", alignItems: "center", gap: 2 }} className="oxplow-md-link">
                <a {...props} onClick={handleLinkClick} onAuxClick={handleLinkClick} />
                {items.length > 0 ? (
                  <span className="oxplow-md-link-kebab" style={{ display: "inline-flex" }}>
                    <Kebab items={items} size={12} label="Link actions" />
                  </span>
                ) : null}
              </span>
            );
          },
        }}
      >
        {processedBody}
      </ReactMarkdown>
    </div>
  );
}
