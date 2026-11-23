import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { MediaLightbox, type LightboxContent } from "./MediaLightbox.js";

import { useRowContextMenu } from "../useRowContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { PageKindIcon } from "../../pageKinds.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { WORKING } from "../../revision.js";
import { useRefTitle } from "../../refTitleCache.js";
import { useRecordStores } from "../../recordStoresLoader.js";
import { useResources } from "../../resources.js";
import { parseMarkdownLink, parsedLinkIconKind, preprocessWikilinks, refLinkTitle, titledRef, urlTransform } from "./markdownLinks.js";
import { attachPanZoom, loadMermaid } from "./mermaidRender.js";
import { CopyCodeButton } from "./CopyCodeButton.js";
import { rehypeHighlightCode } from "./codeHighlight.js";
import { InlineView } from "./InlineView.js";
import { INLINE_ELEMENTS } from "./inlineMarkdown.js";
import { followLink, linkMenuAtOpen, type OpenRef } from "./links.js";
import { readBytesAt } from "../../api.js";
import { mediaType } from "../../filesystem.js";
import { useVizStream } from "../../viz/VizStream.js";

// Mermaid + svg-pan-zoom load lazily inside mermaidRender, so this module
// stays safe to import in non-DOM test environments.

/**
 * Inline anchor wrapper that owns the page-title swap. Lives as its
 * own component so `useRefLinkTitle` (a hook) can be called per-link
 * without violating rules-of-hooks in the parent's render-prop.
 */
function RefLinkSpan({
  anchorProps,
  handleLinkClick,
  items,
  iconKind,
  titledRef: ref,
}: {
  anchorProps: React.AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown };
  handleLinkClick: (event: React.MouseEvent<HTMLAnchorElement>) => void;
  items: () => Promise<MenuItem[]>;
  iconKind: string | null;
  /** The ref the link names when it has a title: read from its resource's dataset. */
  titledRef: string | null;
}) {
  const cm = useRowContextMenu(items);
  const { children, ...rest } = anchorProps;
  const { title: overrideText, brokenReason } = useRefLinkTitle(ref, flattenChildrenText(children));
  if (brokenReason) {
    return <BrokenLink reason={brokenReason}>{children}</BrokenLink>;
  }
  return (
    <span
      style={{ display: "inline-flex", alignItems: "center", gap: 3 }}
      className="oxplow-md-link"
      onContextMenu={cm.onContextMenu}
    >
      {iconKind ? (
        <PageKindIcon
          kind={iconKind}
          size={12}
          style={{ color: "var(--text-secondary)", flexShrink: 0, verticalAlign: "middle" }}
        />
      ) : null}
      <a {...rest} onClick={handleLinkClick} onAuxClick={handleLinkClick}>
        {overrideText ?? children}
      </a>
      {cm.menu}
    </span>
  );
}

/**
 * The one title rule for a ref link, the viewer's and the editor's: the
 * title replaces the link's text when the link was written bare (its text
 * still equals the slug / id, no `|label`) and the title resolved; an
 * author's label and a ref not yet loaded keep the text. A ref whose
 * object doesn't exist (a deleted page, an item not on the active list)
 * is broken, with the reason. `loading` and `found` both stay a live
 * link, so a ref not yet resolved isn't briefly flagged.
 */
export function useRefLinkTitle(ref: string | null, text: string): { title: string | null; brokenReason: string | null } {
  return refLinkTitle(ref, text, useRefTitle(ref));
}

/**
 * An image a markdown file names by a relative path: read from the file's
 * filesystem at its revision (`readBytesAt`) and drawn from a blob URL,
 * given back when the image goes. A file that isn't there, or isn't an
 * image, keeps its alt text, as a browser shows a missing image.
 */
function FileImage({
  path,
  version,
  filesystem,
  alt,
  zoom,
}: {
  path: string;
  version: import("../../revision.js").Revision | null;
  filesystem: string | null;
  alt: string;
  zoom: (shown: string) => { onClick: (e: React.MouseEvent) => void; style: CSSProperties };
}) {
  const stream = useVizStream();
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    let cancelled = false;
    let made: string | null = null;
    setUrl(null);
    const type = mediaType(path);
    if (type?.kind !== "image") return;
    void readBytesAt(stream?.id ?? "", path, { ...(filesystem ? { filesystem } : {}), ...(version && version !== WORKING ? { rev: version } : {}) })
      .then((bytes) => {
        if (cancelled || !bytes) return;
        made = URL.createObjectURL(new Blob([bytes as BlobPart], { type: type.mime }));
        setUrl(made);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (made) URL.revokeObjectURL(made);
    };
  }, [stream?.id, path, version, filesystem]);
  if (!url) return <img alt={alt} title={path} />;
  return <img alt={alt} title={path} src={url} {...zoom(url)} />;
}

/**
 * A broken wikilink — an unrecognized target (`[[#13]]`) or a recognized
 * ref whose object doesn't exist. Rendered as inert text (no anchor, no
 * click handler) with a tooltip explaining why, so a reader can't click
 * through to nowhere and the author sees it needs fixing.
 */
export function BrokenLink({ reason, children }: { reason: string; children: React.ReactNode }) {
  return (
    <span className="oxplow-broken-link" data-testid="broken-wikilink" title={reason}>
      {children}
    </span>
  );
}

function flattenChildrenText(children: React.ReactNode): string {
  if (typeof children === "string") return children;
  if (typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(flattenChildrenText).join("");
  return "";
}

export interface MarkdownViewProps {
  body: string;
  /** The URI of the file the body came from: its relative links and
   *  images are that file's folder's, on its filesystem and revision. */
  base?: string | null;
  /** Apply max-height + internal scroll instead of growing unbounded. */
  maxHeight?: number | string;
  /** Extra style overrides for the outer wrapper. */
  style?: CSSProperties;
  className?: string;
  /** One line: only inline marks and links (`INLINE_ELEMENTS`), blocks
   *  unwrapped, in a `span` that flows in its text. */
  inline?: boolean;
  /** `data-testid` on its root. */
  testId?: string;
}

/**
 * The markdown viewer: the read mode of `Markdown` (`Markdown.tsx`), the
 * one component every surface draws markdown with. Sanitization is
 * react-markdown + remark-gfm's (raw HTML is stripped: no scripts, no
 * fetches beyond links and images).
 *
 * Links open as the editor's do (`links.ts`): through the page's
 * navigation, Cmd/Ctrl-click or middle-click in a new tab, a web link in
 * the app when the allowlist takes it; an in-page anchor jumps. Mermaid
 * fences draw as diagrams (pan and zoom, the lightbox); other code blocks
 * carry a Copy button.
 */
export function MarkdownView({ body, base = null, maxHeight, style, className, inline = false, testId }: MarkdownViewProps) {
  // Extensions' ref kinds change what a `[[…]]` names.
  const refKinds = useResources();
  // …and the stores of the shared models (their ids) do too.
  const stores = useRecordStores();
  const processedBody = useMemo(
    () => preprocessWikilinks(body),
    [body, refKinds, stores],
  );
  const ref = useRef<HTMLDivElement & HTMLSpanElement>(null);
  const [lightbox, setLightbox] = useState<LightboxContent | null>(null);
  // Keep a stable handle to setLightbox so the imperative mermaid
  // effect can read it without re-running on every render.
  const openLightboxRef = useRef<(content: LightboxContent) => void>(() => {});
  openLightboxRef.current = setLightbox;
  const ctxNav = useOptionalPageNavigation();
  const open: OpenRef = ctxNav?.openRef ? ctxNav.openRef.bind(ctxNav) : null;

  const handleLinkClick = useCallback((event: React.MouseEvent<HTMLAnchorElement>) => {
    const href = event.currentTarget.getAttribute("href") ?? "";
    // Cmd/Ctrl-click and middle-click mean "open it over there, keep
    // what I'm reading" — the browser convention.
    const newTab = event.metaKey || event.ctrlKey || event.button === 1;
    if (followLink(href, open, newTab, base)) event.preventDefault();
    // `open` is rebuilt per render from ctxNav.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctxNav, base]);

  // Mermaid rendering pass. Replaces
  // <pre><code class="language-mermaid">…</code></pre> blocks with SVG.
  // Each rendered SVG is wrapped in svg-pan-zoom + a small overlay
  // toolbar (+ / − / Reset) so large diagrams aren't dead space.
  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const cleanups: Array<() => void> = [];
    let cancelled = false;
    const blocks = root.querySelectorAll<HTMLElement>("code.language-mermaid");
    blocks.forEach(async (code, idx) => {
      const source = code.textContent ?? "";
      const id = `mermaid-${Date.now()}-${idx}`;
      const sweepStrayMermaidNodes = () => {
        // Mermaid v11 sometimes appends a temp container to
        // `document.body` with our id (or `d<id>`) and, on parse
        // failure, may inject its default error renderer (the
        // "Syntax error in text" bomb SVG) into that container.
        // Sweep both shapes so the bomb doesn't linger outside our
        // inline error in the markdown view.
        for (const stray of document.querySelectorAll(
          `#${CSS.escape(id)}, #d${CSS.escape(id)}`,
        )) {
          // Don't yank the diagram we just successfully spliced into
          // the markdown view — that one lives under our `root`.
          if (stray.closest(".mermaid-rendered")) continue;
          if (root.contains(stray)) continue;
          stray.remove();
        }
      };
      try {
        const mermaid = await loadMermaid();
        // Validate first; parse() has no DOM side effects, so a
        // syntax error throws before render() can drop its bomb into
        // document.body. `suppressErrors: true` would return false
        // instead of throwing, but we want the message in our catch.
        await mermaid.parse(source);
        const { svg } = await mermaid.render(id, source);
        if (cancelled) {
          sweepStrayMermaidNodes();
          return;
        }
        const host = document.createElement("div");
        host.className = "mermaid-rendered";
        host.innerHTML = svg;
        const pre = code.parentElement;
        if (pre && pre.tagName === "PRE") {
          // Insert host as a sibling and hide the <pre>, instead of
          // replacing it. ReactMarkdown still owns the <pre> node;
          // yanking it with replaceWith() leaves React's fiber tree
          // pointing at a detached node, so the next commit crashes
          // in removeChild ("object can not be found here").
          const prevDisplay = (pre as HTMLElement).style.display;
          (pre as HTMLElement).style.display = "none";
          pre.after(host);
          cleanups.push(() => {
            host.remove();
            (pre as HTMLElement).style.display = prevDisplay;
          });
        }
        sweepStrayMermaidNodes();
        const cleanup = await attachPanZoom(host, svg, (html) => {
          openLightboxRef.current({ kind: "svg", html });
        });
        if (cancelled) {
          cleanup?.();
          return;
        }
        if (cleanup) cleanups.push(cleanup);
      } catch (error) {
        sweepStrayMermaidNodes();
        const pre = code.parentElement;
        if (pre && pre.tagName === "PRE") {
          const err = document.createElement("div");
          err.style.color = "var(--severity-critical)";
          err.style.fontSize = "12px";
          err.textContent = `Mermaid parse error: ${String(error)}`;
          pre.after(err);
          // Same reason as the success-path cleanup: this div is a
          // sibling of a React-managed <pre>, so ReactMarkdown's
          // child reconciliation on body change leaves it stranded
          // unless we tear it down explicitly.
          cleanups.push(() => err.remove());
        }
      }
    });
    return () => {
      cancelled = true;
      for (const fn of cleanups) {
        try { fn(); } catch { /* ignore destroy errors */ }
      }
    };
  }, [processedBody]);

  const wrapperStyle: CSSProperties = {
    ...(maxHeight !== undefined ? { maxHeight, overflowY: "auto" } : {}),
    ...style,
  };

  const wrapperClassName = ["oxplow-md", inline ? "oxplow-md-inline" : null, className].filter(Boolean).join(" ");
  const Root = inline ? "span" : "div";

  return (
    <Root ref={ref} className={wrapperClassName} style={wrapperStyle} data-testid={testId}>
      <ReactMarkdown
        {...(inline ? { allowedElements: INLINE_ELEMENTS, unwrapDisallowed: true } : {})}
        remarkPlugins={[remarkGfm]}
        rehypePlugins={[rehypeHighlightCode]}
        urlTransform={urlTransform}
        components={{
          pre: ({ node, children, ...props }) => {
            // A mermaid fence becomes a diagram (the pass above); any
            // other code block carries a Copy of its text.
            const code = node?.children?.[0];
            const classes = code && "properties" in code ? (code.properties?.className as string[] | undefined) : undefined;
            if (classes?.includes("language-oxplow-page")) {
              const source = code && "children" in code ? code.children.map((child) => "value" in child ? child.value : "").join("") : "";
              return <InlineView specYaml={source} />;
            }
            if (classes?.includes("language-mermaid")) return <pre {...props}>{children}</pre>;
            return (
              <div className="oxplow-md-code">
                <CopyCodeButton read={(button) => button.parentElement?.querySelector("pre")?.textContent ?? ""} />
                <pre {...props}>{children}</pre>
              </div>
            );
          },
          img: ({ node, ...props }) => {
            const src = (props.src as string | undefined) ?? "";
            if (!src) return <img {...props} />;
            const zoom = (shown: string) => ({
              onClick: (e: React.MouseEvent) => {
                e.preventDefault();
                setLightbox({ kind: "image", src: shown, alt: (props.alt as string | undefined) ?? "" });
              },
              style: { ...(props.style as CSSProperties | undefined), cursor: "zoom-in", maxWidth: "100%" },
            });
            // A relative image of a file is read from that file's folder.
            const inFile = parseMarkdownLink(src, base);
            if (base && inFile.kind === "file") {
              return <FileImage path={inFile.path} version={inFile.version} filesystem={inFile.filesystem ?? null} alt={(props.alt as string | undefined) ?? ""} zoom={zoom} />;
            }
            return <img {...props} {...zoom(src)} />;
          },
          a: ({ node, ...props }) => {
            const href = (props.href as string | undefined) ?? "";
            const parsed = parseMarkdownLink(href, base);
            // Unrecognized-syntax targets (`[[#13]]`) render broken and
            // non-clickable, with the reason in a tooltip.
            if (parsed.kind === "broken") {
              return <BrokenLink reason={parsed.reason}>{props.children}</BrokenLink>;
            }
            // Anchor and empty links don't get a kebab — there's no useful
            // action besides "jump in page" for those.
            if (parsed.kind === "anchor" || parsed.kind === "empty") {
              return <a {...props} onClick={handleLinkClick} onAuxClick={handleLinkClick} />;
            }
            const items = () => linkMenuAtOpen(href, open, base);
            const iconKind = parsedLinkIconKind(parsed);
            // For links written bare (`[[slug]]`, no `|label`), swap the
            // rendered text from the id to the row's title so readers see
            // "Local Snapshots" not `local-snapshots`. Author-supplied
            // labels are preserved.
            return (
              <RefLinkSpan
                anchorProps={props}
                handleLinkClick={handleLinkClick}
                items={items}
                iconKind={iconKind}
                titledRef={titledRef(parsed)}
              />
            );
          },
        }}
      >
        {processedBody}
      </ReactMarkdown>
      {inline ? null : <MediaLightbox content={lightbox} onClose={() => setLightbox(null)} />}
    </Root>
  );
}
