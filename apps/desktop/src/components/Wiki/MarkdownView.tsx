import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { MediaLightbox, type LightboxContent } from "./MediaLightbox.js";

/**
 * react-markdown's defaultUrlTransform only allows
 * http/https/ircs/mailto/xmpp; our internal schemes (`file:`, `dir:`,
 * `gitcommit:`) get stripped to empty strings, which makes the click
 * handler see `kind: "empty"` and no-op. Pass our schemes through
 * untouched and defer everything else to the default sanitizer.
 */
const APP_SCHEMES = /^(file|dir|gitcommit|task):/i;
function urlTransform(value: string): string {
  if (APP_SCHEMES.test(value)) return value;
  return defaultUrlTransform(value);
}
import { useRowContextMenu } from "../useRowContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { PageKindIcon } from "../../pageKinds.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef, directoryRef, gitCommitRef, wikiPageRef, taskRef } from "../../tabs/pageRefs.js";
import { DISK, type FileVersion } from "../../file-version.js";
import { useWikiTitle } from "../../wikiTitleCache.js";
import { useTaskTitle } from "../../taskTitleCache.js";

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

// svg-pan-zoom uses CommonJS-style `export = svgPanZoom`, so the
// runtime default-import gives us the callable instance directly.
type SvgPanZoomFn = (typeof import("svg-pan-zoom"));
let svgPanZoomPromise: Promise<SvgPanZoomFn> | null = null;
export function loadSvgPanZoom() {
  if (!svgPanZoomPromise) {
    svgPanZoomPromise = import("svg-pan-zoom").then((mod) => {
      // Vite's CJS interop wraps the export under `.default`; native
      // ESM gives the function directly. Handle both shapes.
      const m = mod as unknown as { default?: SvgPanZoomFn };
      return (m.default ?? (mod as unknown as SvgPanZoomFn));
    });
  }
  return svgPanZoomPromise;
}

/**
 * Wrap a freshly rendered Mermaid host in svg-pan-zoom and inject a
 * small overlay toolbar (+ / − / Reset). Returns a cleanup that tears
 * the pan-zoom instance down — important so the React effect can
 * dispose stale instances when the body re-renders.
 *
 * Mermaid emits an `<svg>` whose width/height come from the diagram's
 * intrinsic size; svg-pan-zoom needs an explicit fixed size on the
 * element so the viewport math works. We give the wrapper a fixed
 * height and let the SVG fill it.
 */
/**
 * Wait for the host to have a non-zero layout box. b775f12 mounts
 * back/forward history tabs as `display:none` siblings, so this
 * effect can fire while the host is laid out at 0×0; svg-pan-zoom
 * then calls `getCTM().inverse()` on a zero-size SVG and throws
 * `InvalidStateError: Matrix is not invertible`, leaving the diagram
 * permanently blank. IntersectionObserver fires when the host first
 * intersects the viewport; until then we defer init.
 */
function waitForVisible(host: HTMLElement): Promise<void> {
  if (host.offsetWidth > 0 && host.offsetHeight > 0) return Promise.resolve();
  return new Promise((resolve) => {
    const obs = new IntersectionObserver((entries) => {
      for (const entry of entries) {
        if (entry.isIntersecting && host.offsetWidth > 0 && host.offsetHeight > 0) {
          obs.disconnect();
          resolve();
          return;
        }
      }
    });
    obs.observe(host);
  });
}

async function attachPanZoom(host: HTMLElement, onExpand?: () => void): Promise<(() => void) | null> {
  const svg = host.querySelector<SVGSVGElement>("svg");
  if (!svg) return null;
  // svg-pan-zoom requires the SVG to have a width/height set.
  svg.removeAttribute("style");
  svg.setAttribute("width", "100%");
  svg.setAttribute("height", "100%");
  host.style.position = "relative";
  host.style.height = "480px";
  host.style.maxHeight = "70vh";
  host.style.border = "1px solid var(--border-subtle)";
  host.style.borderRadius = "6px";
  host.style.overflow = "hidden";
  // Block until the host has real dimensions — see waitForVisible
  // comment for the b775f12 hidden-tab interaction.
  await waitForVisible(host);
  const svgPanZoom = await loadSvgPanZoom();
  const instance = svgPanZoom(svg, {
    zoomEnabled: true,
    // Mouse-wheel zoom is hostile inside a scrollable wiki page —
    // users expect the wheel to scroll the article, not silently
    // resize the diagram. The +/− toolbar buttons drive zoom.
    mouseWheelZoomEnabled: false,
    panEnabled: true,
    controlIconsEnabled: false,
    fit: true,
    center: true,
    minZoom: 0.2,
    maxZoom: 20,
    contain: false,
  });
  const toolbar = document.createElement("div");
  toolbar.className = "mermaid-pz-toolbar";
  toolbar.style.cssText = [
    "position: absolute",
    "top: 6px",
    "right: 6px",
    "display: flex",
    "gap: 2px",
    "background: var(--surface-card)",
    "border: 1px solid var(--border-subtle)",
    "border-radius: 4px",
    "padding: 2px",
    "font-size: 12px",
    "z-index: 1",
  ].join(";");
  const makeBtn = (label: string, title: string, onClick: () => void) => {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = label;
    btn.title = title;
    btn.style.cssText = [
      "background: transparent",
      "border: none",
      "color: var(--text-primary)",
      "cursor: pointer",
      "padding: 2px 6px",
      "font-size: 12px",
      "line-height: 1",
      "min-width: 20px",
    ].join(";");
    btn.addEventListener("click", (e) => { e.preventDefault(); onClick(); });
    return btn;
  };
  toolbar.appendChild(makeBtn("−", "Zoom out", () => instance.zoomOut()));
  toolbar.appendChild(makeBtn("+", "Zoom in", () => instance.zoomIn()));
  toolbar.appendChild(makeBtn("Reset", "Reset view", () => { instance.resetZoom(); instance.center(); instance.fit(); }));
  if (onExpand) {
    toolbar.appendChild(makeBtn("⛶", "Open in lightbox", () => onExpand()));
  }
  host.appendChild(toolbar);
  return () => {
    try { instance.destroy(); } catch { /* ignore */ }
    toolbar.remove();
  };
}

export type ParsedLink =
  | { kind: "empty" }
  | { kind: "anchor" }
  | { kind: "external" }
  | { kind: "internal"; slug: string }
  | {
      kind: "file";
      path: string;
      line?: number;
      /** Tree version the wikilink pinned. `null` means the wikilink
       *  was bare (`[[path]]`) — host falls back to `DISK` (working
       *  tree) for back-compat. Non-null carries the author's intent
       *  exactly. */
      version: import("../../file-version.js").FileVersion | null;
    }
  | { kind: "directory"; path: string }
  | { kind: "git-commit"; sha: string }
  | { kind: "task"; id: string };

const SHA_RE = /^[0-9a-f]{7,40}$/i;

/**
 * Heuristic: does a wikilink target look like a git commit reference?
 * Either the explicit `git:<sha>` form or a bare 7-40 char hex string.
 * Bare-hex detection is safe alongside file paths because file targets
 * always carry a slash or recognizable extension; safe alongside note
 * slugs because slugs are kebab-case English words, not hex.
 */
export function parseGitRefTarget(target: string): string | null {
  const stripped = target.startsWith("git:") ? target.slice(4) : target;
  if (!SHA_RE.test(stripped)) return null;
  return stripped.toLowerCase();
}

/**
 * Classify a markdown link href. Shared by WikiPageTab (wiki navigation) and
 * TaskDetail (tasks modal markdown rendering). Pure — easy to test.
 */
/**
 * Map a `ParsedLink.kind` to the unified scheme/kind string the
 * shared `PageKindIcon` understands. Returns `null` for kinds that
 * shouldn't carry a leading glyph.
 */
function parsedLinkIconKind(kind: ParsedLink["kind"]): string | null {
  switch (kind) {
    case "internal":
      return "wiki";
    case "file":
      return "file";
    case "directory":
      return "directory";
    case "git-commit":
      return "git-commit";
    case "task":
      return "task";
    case "external":
      return "external-url";
    default:
      return null;
  }
}

export function parseMarkdownLink(rawHref: string): ParsedLink {
  if (!rawHref) return { kind: "empty" };
  if (rawHref.startsWith("#")) return { kind: "anchor" };
  if (/^https?:\/\//i.test(rawHref) || rawHref.startsWith("mailto:")) {
    return { kind: "external" };
  }
  if (rawHref.startsWith("file:")) {
    const raw = rawHref.slice("file:".length);
    if (!raw) return { kind: "empty" };
    // The preprocessed href shape is `file:<path>[@<version>][:<line>]`.
    // Pull the version off first since it can contain hex / branch
    // names that the line regex would mishandle.
    let body = raw;
    let version: import("../../file-version.js").FileVersion | null = null;
    const atIdx = body.indexOf("@");
    if (atIdx > 0) {
      const versionPart = body.slice(atIdx + 1);
      body = body.slice(0, atIdx);
      // Version may have a trailing `:<line>` anchor.
      const colonIdx = versionPart.lastIndexOf(":");
      let versionToken = versionPart;
      let trailingLine: string | null = null;
      if (colonIdx >= 0) {
        const maybeLine = versionPart.slice(colonIdx + 1);
        if (/^\d+$/.test(maybeLine)) {
          versionToken = versionPart.slice(0, colonIdx);
          trailingLine = maybeLine;
        }
      }
      if (versionToken.toLowerCase() === "disk" || versionToken.toLowerCase() === "local") {
        version = { kind: "disk" };
      } else if (versionToken) {
        version = { kind: "ref", ref: versionToken };
      }
      if (trailingLine != null) {
        body = `${body}:${trailingLine}`;
      }
    }
    const lineMatch = body.match(/^(.+?):(\d+)$/);
    if (lineMatch) {
      return { kind: "file", path: lineMatch[1]!, line: Number(lineMatch[2]), version };
    }
    return { kind: "file", path: body, version };
  }
  if (rawHref.startsWith("dir:")) {
    const raw = rawHref.slice("dir:".length).replace(/\/+$/, "");
    if (!raw) return { kind: "empty" };
    return { kind: "directory", path: raw };
  }
  if (rawHref.startsWith("gitcommit:")) {
    const sha = rawHref.slice("gitcommit:".length);
    if (!sha) return { kind: "empty" };
    return { kind: "git-commit", sha };
  }
  if (rawHref.startsWith("task:")) {
    const id = rawHref.slice("task:".length);
    if (!id) return { kind: "empty" };
    return { kind: "task", id };
  }
  let target = rawHref.replace(/^\.?\//, "");
  target = target.split("#")[0]?.split("?")[0] ?? "";
  if (target.endsWith(".md")) target = target.slice(0, -3);
  return target ? { kind: "internal", slug: target } : { kind: "empty" };
}

/**
 * Inverse of `preprocessWikilinks` — converts standard markdown links
 * with our internal URL schemes back into `[[ ]]` wikilink form so the
 * on-disk file keeps its authored shape after a round-trip through the
 * RichText editor.
 *
 * Mappings:
 * - `[label](file:path)`        → `[[path]]`         if label === path
 *                              → `[[path|label]]`   otherwise
 * - `[label](dir:path)`         → `[[dir:path]]`     if label === path
 *                              → `[[dir:path|label]]` otherwise
 * - `[label](gitcommit:sha)`    → `[[git:sha]]`      if label is the
 *                                                    7-char-or-longer
 *                                                    hex prefix of sha
 *                              → `[[git:sha|label]]` otherwise
 *
 * Plain markdown links (http/https/internal wiki slug with no scheme,
 * etc.) are left alone — round-tripping those into `[[ ]]` would be
 * lossy and ambiguous. Image links (`![alt](url)`) are skipped.
 *
 * Fenced code blocks and inline code spans are protected, matching
 * `preprocessWikilinks`'s skip pattern.
 */
export function postprocessWikilinks(body: string): string {
  const segments = body.split(/(```[\s\S]*?```)/g);
  return segments.map((seg, idx) => {
    if (idx % 2 === 1) return seg;
    return collapseLinksOutsideInlineCode(seg);
  }).join("");
}

function collapseLinksOutsideInlineCode(text: string): string {
  const parts = text.split(/(`[^`\n]*`)/g);
  return parts.map((part, idx) => {
    if (idx % 2 === 1) return part;
    // First clean up the editor's escaped-bracket round-trip of bare
    // wikilinks: markdown-it rejects `file:` URLs in its default
    // `validateLink`, so `[label](file:path)` ends up stored as
    // literal text in Tiptap; on serialize the outer brackets get
    // escaped, leaving `\[[target|label\]]` on disk. Strip that
    // escaping back to a real wikilink before everything else runs.
    const unmangled = part.replace(
      /\\\[\[([^\]\n|]+)\|([^\]\n]+)\\\]\]/g,
      (_m, target: string, label: string) =>
        target === label ? `[[${target}]]` : `[[${target}|${label}]]`,
    );
    // Then collapse `[label](url)` for our internal schemes — but
    // NOT `![alt](url)` (images).
    return unmangled.replace(
      /(^|[^!])\[([^\]\n]+)\]\(((?:file|dir|gitcommit|task):[^)\s]+)\)/g,
      (_match, lead: string, label: string, url: string) => {
        const collapsed = collapseInternalLink(label, url);
        return collapsed == null ? _match : `${lead}${collapsed}`;
      },
    );
  }).join("");
}

function collapseInternalLink(label: string, url: string): string | null {
  if (url.startsWith("file:")) {
    const path = url.slice("file:".length);
    if (!path) return null;
    return label === path ? `[[${path}]]` : `[[${path}|${label}]]`;
  }
  if (url.startsWith("dir:")) {
    const path = url.slice("dir:".length);
    if (!path) return null;
    return label === path ? `[[dir:${path}]]` : `[[dir:${path}|${label}]]`;
  }
  if (url.startsWith("gitcommit:")) {
    const sha = url.slice("gitcommit:".length);
    if (!sha) return null;
    // If the label is a prefix of the SHA (the bare `[[<sha>]]` form
    // renders as the short 7-char SHA), drop the label and emit the
    // bare wikilink. Otherwise preserve it.
    const labelIsHexPrefix = /^[0-9a-f]{7,40}$/i.test(label) && sha.toLowerCase().startsWith(label.toLowerCase());
    return labelIsHexPrefix ? `[[git:${sha}]]` : `[[git:${sha}|${label}]]`;
  }
  if (url.startsWith("task:")) {
    const id = url.slice("task:".length);
    if (!id) return null;
    // Bare `[[tsk42]]` renders with the id as text (the title swap is a
    // render-time overlay, not stored), so a label equal to the id
    // collapses to the bare form.
    return label === id ? `[[${id}]]` : `[[${id}|${label}]]`;
  }
  return null;
}

/**
 * Heuristic: does a wikilink target look like a repo file path rather
 * than a wiki note slug? File paths contain a slash or end in a recognizable
 * extension; bare slugs like `architecture` are routed to wiki navigation.
 */
function looksLikeFilePath(target: string): boolean {
  // Strip a trailing `@<version>` segment before the heuristic
  // checks; the version isn't part of the path's slash/extension
  // shape but does sit on the same token.
  const atIdx = target.indexOf("@");
  const path = atIdx > 0 ? target.slice(0, atIdx) : target;
  if (path.includes("/")) return true;
  // Tail extension other than .md → file. .md → wiki note.
  const dot = path.lastIndexOf(".");
  if (dot <= 0) return false;
  const ext = path.slice(dot + 1).toLowerCase();
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
      const sha = parseGitRefTarget(target);
      if (sha) {
        // Display short sha when the user didn't supply a label and the
        // raw target is the full hex (avoid 40-char inline link text).
        const shortDisplay = label ? display : sha.slice(0, 7);
        return `[${shortDisplay}](gitcommit:${sha})`;
      }
      // Directory form: explicit `dir:` prefix (mirrors `git:`).
      // Trailing slash on the path is tolerated and stripped.
      if (target.startsWith("dir:")) {
        const dir = target.slice(4).trim().replace(/\/+$/, "");
        if (dir) {
          // When no label was supplied, default to showing the bare
          // path (without the `dir:` prefix) so the rendered link
          // reads as a directory rather than a URL-shaped token.
          const dirDisplay = label ? display : dir;
          return `[${dirDisplay}](dir:${dir})`;
        }
      }
      // Task ref: `[[tsk<digits>]]` (the whole token is the task id). The
      // rendered link text defaults to the token; the task title is swapped
      // in at render time (WikiLinkSpan + useTaskTitle). Matches the backend
      // ref extractor (refs.rs), which recognizes the same `tsk<digits>`
      // form for backlinks.
      if (/^tsk\d+$/i.test(target)) {
        return `[${display}](task:${target})`;
      }
      if (looksLikeFilePath(target)) {
        // The target may carry a `@<version>` segment; the file:
        // URL preserves it verbatim and parseMarkdownLink decodes it
        // back into a FileVersion at click time.
        return `[${display}](file:${target})`;
      }
      // Treat as wiki note slug.
      return `[${display}](${target})`;
    });
  }).join("");
}

/**
 * Inline anchor wrapper that owns the wiki-title swap. Lives as its
 * own component so `useWikiTitle` (a hook) can be called per-link
 * without violating rules-of-hooks in the parent's render-prop.
 */
function WikiLinkSpan({
  anchorProps,
  handleLinkClick,
  items,
  iconKind,
  internalSlug,
  taskId,
}: {
  anchorProps: React.AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown };
  handleLinkClick: (event: React.MouseEvent<HTMLAnchorElement>) => void;
  items: MenuItem[];
  iconKind: string | null;
  internalSlug: string | null;
  taskId: string | null;
}) {
  const title = useWikiTitle(internalSlug);
  const taskTitle = useTaskTitle(taskId);
  const cm = useRowContextMenu(items);
  const { children, ...rest } = anchorProps;
  // The link text is swapped for the resolved title when the link was
  // authored as a bare ref (the rendered text still equals the slug / id,
  // i.e. no `|label`) and the cache resolved a real title. Author-supplied
  // labels (`[[slug|Custom]]`) and unknown / not-yet-loaded refs fall back
  // to the original children (the slug / id) verbatim.
  const childrenText = flattenChildrenText(children);
  const overrideText =
    internalSlug && title && childrenText === internalSlug
      ? title
      : taskId && taskTitle && childrenText === taskId
        ? taskTitle
        : null;
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

function flattenChildrenText(children: React.ReactNode): string {
  if (typeof children === "string") return children;
  if (typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(flattenChildrenText).join("");
  return "";
}

export interface MarkdownViewProps {
  body: string;
  /** Optional internal link handler (WikiPageTab routes to wiki history). */
  onNavigateInternal?: (slug: string) => void;
  /** Optional new-tab handler (WikiPageTab opens slug in another tab). */
  onOpenInNewTab?: (slug: string) => void;
  /** Optional file-link handler — invoked for `[[path/to/file]]` wikilinks. */
  onOpenFile?: (path: string, line?: number) => void;
  /** Optional directory-link handler — invoked for `[[path/to/dir/]]` wikilinks. */
  onOpenDirectory?: (path: string) => void;
  /** Optional git-commit-link handler — invoked for `[[<sha>]]` / `[[git:<sha>]]` wikilinks. */
  onOpenCommit?: (sha: string) => void;
  /**
   * Optional handler for external (http/https) link clicks. When present,
   * left-click on an external link calls this instead of opening in the
   * OS browser; the host wires it to "open as in-app external-url tab".
   * Right-click "Open in browser" still goes to the OS browser regardless.
   */
  onOpenExternalUrl?: (url: string) => void;
  /**
   * Render mermaid code blocks as SVG diagrams. WikiPageTab uses this; the
   * tasks modal disables it (default false) since tasks notes
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
 * Plan tasks modal. Sanitization is delegated to react-markdown +
 * remark-gfm, which strip raw HTML by default — no scripts, no
 * arbitrary external fetches beyond standard markdown links/images.
 *
 * Link behavior:
 * - external links open in the OS browser (Electron `_blank`).
 * - anchor (`#…`) links use default behavior (in-page jump).
 * - internal links route through `onNavigateInternal` / `onOpenInNewTab`
 *   when those handlers are supplied; otherwise they no-op (tasks
 *   notes don't have a wiki to navigate to).
 */
export function MarkdownView({
  body,
  onNavigateInternal,
  onOpenInNewTab,
  onOpenFile,
  onOpenDirectory,
  onOpenCommit,
  onOpenExternalUrl,
  renderMermaid = false,
  maxHeight,
  style,
  className,
}: MarkdownViewProps) {
  const processedBody = useMemo(() => preprocessWikilinks(body), [body]);
  const ref = useRef<HTMLDivElement | null>(null);
  const [lightbox, setLightbox] = useState<LightboxContent | null>(null);
  // Keep a stable handle to setLightbox so the imperative mermaid
  // effect can read it without re-running on every render.
  const openLightboxRef = useRef<(content: LightboxContent) => void>(() => {});
  openLightboxRef.current = setLightbox;
  // Page-context chokepoint: when this MarkdownView renders inside a
  // page, plain-click follows browser-tab semantics (in-tab nav).
  // Modifier-click + middle/right-click always escape to a new tab.
  // Outside a page (e.g. tasks modal), the host callbacks own
  // the click.
  const ctxNav = useOptionalPageNavigation();

  const handleLinkClick = useCallback((event: React.MouseEvent<HTMLAnchorElement>) => {
    const href = event.currentTarget.getAttribute("href") ?? "";
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "anchor") return;
    event.preventDefault();
    if (parsed.kind === "empty") return;
    if (parsed.kind === "external") {
      if (onOpenExternalUrl) onOpenExternalUrl(href);
      else window.open(href, "_blank", "noopener,noreferrer");
      return;
    }
    const newTab = event.metaKey || event.ctrlKey || event.button === 1;
    if (parsed.kind === "file") {
      // Bare wikilinks (no `@version`) coerce to DISK; explicit
      // versions flow through as authored. The chokepoint here is
      // critical: a wikilink that pinned `@HEAD` must NOT be
      // silently substituted with the working tree.
      const version: FileVersion = parsed.version ?? DISK;
      if (ctxNav && !newTab) {
        ctxNav.navigate(fileRef(parsed.path, version), { newTab: false });
        return;
      }
      onOpenFile?.(parsed.path, parsed.line);
      return;
    }
    if (parsed.kind === "directory") {
      if (ctxNav && !newTab) {
        ctxNav.navigate(directoryRef(parsed.path), { newTab: false });
        return;
      }
      onOpenDirectory?.(parsed.path);
      return;
    }
    if (parsed.kind === "git-commit") {
      if (ctxNav && !newTab) {
        ctxNav.navigate(gitCommitRef(parsed.sha), { newTab: false });
        return;
      }
      onOpenCommit?.(parsed.sha);
      return;
    }
    if (parsed.kind === "task") {
      // Task links route through the page-nav chokepoint (present on wiki
      // and task pages). No legacy callback fallback — tasks weren't a
      // wikilink target before.
      if (ctxNav) ctxNav.navigate(taskRef(parsed.id), { newTab });
      return;
    }
    // Internal (wiki) link
    if (ctxNav && !newTab) {
      ctxNav.navigate(wikiPageRef(parsed.slug), { newTab: false });
      return;
    }
    if (newTab && onOpenInNewTab) onOpenInNewTab(parsed.slug);
    else if (onNavigateInternal) onNavigateInternal(parsed.slug);
    // No handlers? Silently ignore — tasks notes don't have wiki nav.
  }, [ctxNav, onNavigateInternal, onOpenInNewTab, onOpenFile, onOpenDirectory, onOpenCommit, onOpenExternalUrl]);

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
      const items: MenuItem[] = [];
      if (onOpenExternalUrl) {
        items.push({ id: "open-in-app", label: "Open in app", enabled: true, run: () => onOpenExternalUrl(href) });
      }
      items.push({ id: "open-ext", label: "Open in browser", enabled: true, run: () => { window.open(href, "_blank", "noopener,noreferrer"); } });
      items.push({ id: "copy", label: "Copy link", enabled: true, run: () => { void navigator.clipboard.writeText(href).catch(() => {}); } });
      return items;
    }
    if (parsed.kind === "file") {
      const items: MenuItem[] = [];
      if (onOpenFile) {
        items.push({ id: "open-file", label: "Open file", enabled: true, run: () => onOpenFile(parsed.path, parsed.line) });
      }
      items.push({ id: "copy-path", label: "Copy path", enabled: true, run: () => { void navigator.clipboard.writeText(parsed.path).catch(() => {}); } });
      return items;
    }
    if (parsed.kind === "directory") {
      const items: MenuItem[] = [];
      if (onOpenDirectory) {
        items.push({ id: "open-dir", label: "Open directory", enabled: true, run: () => onOpenDirectory(parsed.path) });
      }
      items.push({ id: "copy-path", label: "Copy path", enabled: true, run: () => { void navigator.clipboard.writeText(parsed.path).catch(() => {}); } });
      return items;
    }
    if (parsed.kind === "git-commit") {
      const items: MenuItem[] = [];
      if (onOpenCommit) {
        items.push({ id: "open-commit", label: "Open commit", enabled: true, run: () => onOpenCommit(parsed.sha) });
      }
      items.push({ id: "copy-sha", label: "Copy SHA", enabled: true, run: () => { void navigator.clipboard.writeText(parsed.sha).catch(() => {}); } });
      return items;
    }
    if (parsed.kind === "task") {
      const id = parsed.id;
      const items: MenuItem[] = [];
      if (ctxNav) {
        items.push({ id: "open", label: "Open", enabled: true, run: () => ctxNav.navigate(taskRef(id), { newTab: false }) });
        items.push({ id: "open-new", label: "Open in new tab", enabled: true, run: () => ctxNav.navigate(taskRef(id), { newTab: true }) });
      }
      items.push({ id: "copy-id", label: "Copy task id", enabled: true, run: () => { void navigator.clipboard.writeText(id).catch(() => {}); } });
      return items;
    }
    return [];
  }, [ctxNav, onNavigateInternal, onOpenInNewTab, onOpenFile, onOpenDirectory, onOpenCommit, onOpenExternalUrl]);

  // Mermaid rendering pass — opt-in via renderMermaid flag. Replaces
  // <pre><code class="language-mermaid">…</code></pre> blocks with SVG.
  // Each rendered SVG is wrapped in svg-pan-zoom + a small overlay
  // toolbar (+ / − / Reset) so large diagrams aren't dead space.
  useEffect(() => {
    if (!renderMermaid) return;
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
        const cleanup = await attachPanZoom(host, () => {
          openLightboxRef.current({ kind: "svg", html: svg });
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
  }, [body, renderMermaid]);

  const wrapperStyle: CSSProperties = {
    ...(maxHeight !== undefined ? { maxHeight, overflowY: "auto" } : {}),
    ...style,
  };

  const wrapperClassName = ["oxplow-md", className].filter(Boolean).join(" ");

  return (
    <div ref={ref} className={wrapperClassName} style={wrapperStyle}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={urlTransform}
        components={{
          img: ({ node, ...props }) => {
            const src = (props.src as string | undefined) ?? "";
            if (!src) return <img {...props} />;
            return (
              <img
                {...props}
                onClick={(e) => {
                  e.preventDefault();
                  setLightbox({ kind: "image", src, alt: (props.alt as string | undefined) ?? "" });
                }}
                style={{ ...(props.style as CSSProperties | undefined), cursor: "zoom-in", maxWidth: "100%" }}
              />
            );
          },
          a: ({ node, ...props }) => {
            const href = (props.href as string | undefined) ?? "";
            const parsed = parseMarkdownLink(href);
            // Anchor and empty links don't get a kebab — there's no useful
            // action besides "jump in page" for those.
            if (parsed.kind === "anchor" || parsed.kind === "empty") {
              return <a {...props} onClick={handleLinkClick} onAuxClick={handleLinkClick} />;
            }
            const items = buildLinkMenu(href);
            const iconKind = parsedLinkIconKind(parsed.kind);
            // For internal wiki links written as bare `[[slug]]` (no
            // `|label`), swap the rendered text from the slug to the
            // page title so readers see "Local Snapshots" not
            // `local-snapshots`. Author-supplied labels are preserved.
            const internalSlug = parsed.kind === "internal" ? parsed.slug : null;
            const taskId = parsed.kind === "task" ? parsed.id : null;
            return (
              <WikiLinkSpan
                anchorProps={props}
                handleLinkClick={handleLinkClick}
                items={items}
                iconKind={iconKind}
                internalSlug={internalSlug}
                taskId={taskId}
              />
            );
          },
        }}
      >
        {processedBody}
      </ReactMarkdown>
      <MediaLightbox content={lightbox} onClose={() => setLightbox(null)} />
    </div>
  );
}
