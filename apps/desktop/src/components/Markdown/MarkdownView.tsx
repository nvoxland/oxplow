import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { CSSProperties } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { MediaLightbox, type LightboxContent } from "./MediaLightbox.js";

/**
 * react-markdown's defaultUrlTransform only allows
 * http/https/ircs/mailto/xmpp; our own scheme (`oxplow:`, every resource's
 * URI; see .context/resources.md) gets stripped to empty strings, which
 * makes the click handler see `kind: "empty"` and no-op. Pass our scheme
 * through untouched and defer everything else to the default sanitizer.
 */
const INVALID_LINK = "oxplow-invalid:";
function urlTransform(value: string): string {
  if (isOxplowScheme(value) || value.startsWith(INVALID_LINK)) return value;
  return defaultUrlTransform(value);
}
import { useRowContextMenu } from "../useRowContextMenu.js";
import type { MenuItem } from "../../menu.js";
import { PageKindIcon } from "../../pageKinds.js";
import { useOptionalPageNavigation } from "../../tabs/PageNavigationContext.js";
import { fileRef, directoryRef, gitCommitRef, wikiPageRef } from "../../tabs/pageRefs.js";
import { WORKING, gitRevision, revisionFromSlot, revisionSlot } from "../../revision.js";
import { useWikiRef } from "../../wikiTitleCache.js";
import { useWorkItemRef } from "../../workItemTitleCache.js";
import { useWorkListProfile } from "../../useWorkListProfile.js";
import { workItemRefOfMention } from "../../workItems.js";
import { workItemId, workItemLabel } from "../../workItemRef.js";
import { dirUri, fileLinesUri, fileUri, identityOf, isOxplowScheme, linkTarget, commitUri, parseUri, typeOf, wikiUri } from "../../uri/uri.js";
import { extensionWikilinkRef, resourceInfo, useExtensionRefTitle, useResources } from "../../resources.js";
import { attachPanZoom, loadMermaid } from "./mermaidRender.js";
import { CopyCodeButton } from "./CopyCodeButton.js";
import { InlineView } from "./InlineView.js";
import { followLink, linkMenu, type OpenRef } from "./links.js";

// Mermaid + svg-pan-zoom load lazily inside mermaidRender, so this module
// stays safe to import in non-DOM test environments (parseMarkdownLink is
// the main reason to import without mounting the component).

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
       *  was bare (`[[path]]`): it names the file as it is now, so the
       *  host opens `WORKING` (the working tree). Non-null carries the
       *  author's intent exactly. */
      version: import("../../revision.js").Revision | null;
    }
  | { kind: "directory"; path: string }
  | { kind: "commit"; sha: string }
  /** A work item, by its URI. */
  | { kind: "work_item"; ref: string }
  /** A URI of an extension's resource (`oxplow:acme/pr?id=12`, P8.D7). */
  | { kind: "ref"; ref: string }
  /** A `[[…]]` whose target matches no known ref shape (e.g. the GitHub
   *  `[[#13]]` form). Rendered as a broken, non-clickable link. */
  | { kind: "broken"; reason: string };

const SHA_RE = /^[0-9a-f]{7,40}$/i;

/** Which work item a loose id in text names, if any: the active list's
 *  ids, as it declares them (`workItemRefOfMention`). */
export type MentionRef = (id: string) => string | null;

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
export function parsedLinkIconKind(parsed: ParsedLink): string | null {
  switch (parsed.kind) {
    case "ref":
      // URI-SWITCH: the icon table is keyed by resource name.
      return typeOf(parsed.ref);
    case "internal":
      return "wiki";
    case "file":
      return "file";
    case "directory":
      return "dir";
    case "commit":
      return "commit";
    case "work_item":
      return "work_item";
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
  const uri = parseUri(rawHref);
  if (uri !== null) {
    const p = uri.params;
    switch (typeOf(rawHref)) {
      case "oxplow/file": {
        if (!p.path) return { kind: "empty" };
        const line = Number(/^(\d+)-\d+$/.exec(uri.view.lines ?? "")?.[1]);
        const version = p.rev ? revisionFromSlot(p.rev) : null;
        return Number.isInteger(line) ? { kind: "file", path: p.path, line, version } : { kind: "file", path: p.path, version };
      }
      case "oxplow/dir": {
        const path = (p.path ?? "").replace(/\/+$/, "");
        return path ? { kind: "directory", path } : { kind: "empty" };
      }
      case "oxplow/commit":
        return p.sha ? { kind: "commit", sha: p.sha } : { kind: "empty" };
      case "oxplow/wiki":
        return p.slug ? { kind: "internal", slug: p.slug } : { kind: "empty" };
      case "oxplow/work_item":
        // Any list's item, by its URI.
        return workItemId(rawHref) === null ? { kind: "empty" } : { kind: "work_item", ref: identityOf(rawHref) ?? rawHref };
      default: {
        // URI-SWITCH: the registry is keyed by resource name (`v_resource`).
        const resource = typeOf(rawHref);
        if (resource && resourceInfo(resource)) return { kind: "ref", ref: rawHref };
      }
    }
  }
  if (rawHref.startsWith(INVALID_LINK)) {
    const target = decodeURIComponent(rawHref.slice(INVALID_LINK.length));
    return {
      kind: "broken",
      reason: `Broken link: “${target}” is not a recognized reference (use a work item's id, [[some-slug]] for a wiki page)`,
    };
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
 * - `[label](commit:sha)`       → `[[git:sha]]`      if label is the
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
      /(^|[^!])\[([^\]\n]+)\]\(([^)\s]+)\)/g,
      (_match, lead: string, label: string, url: string) => {
        const collapsed = isOxplowScheme(url) || url.startsWith(INVALID_LINK) ? collapseInternalLink(label, url) : null;
        return collapsed == null ? _match : `${lead}${collapsed}`;
      },
    );
  }).join("");
}

/**
 * A URI as a link in the editor's spelling — what completion, a drop and a
 * paste put in: `href` its link target, `text` the label
 * `postprocessWikilinks` collapses to the bare wikilink (a wiki page's slug,
 * a file's `path[@rev][:line]`, a directory's path, a commit's short sha, a
 * work item's id when it is the active list's (`mentionRef`), else its
 * whole URI), so it saves as `[[…]]` and shows its title (`useRefLinkTitle`).
 * A ref with no bare form keeps `title` as its label (`[title](uri)`).
 */
export function refLinkOf(uri: string, title: string, mentionRef: MentionRef = () => null): { href: string; text: string } {
  const href = linkTarget(uri);
  const parsed = parseUri(uri);
  const p = parsed?.params ?? {};
  switch (typeOf(uri)) {
    case "oxplow/file": {
      if (!p.path) break;
      const line = /^(\d+)-\d+$/.exec(parsed?.view.lines ?? "")?.[1];
      const rev = p.rev ? `@${p.rev.startsWith("git:") ? p.rev.slice(4) : p.rev}` : "";
      return { href, text: `${p.path}${rev}${line ? `:${line}` : ""}` };
    }
    case "oxplow/dir":
      if (p.path) return { href, text: p.path };
      break;
    case "oxplow/wiki":
      if (p.slug) return { href, text: p.slug };
      break;
    case "oxplow/commit":
      if (p.sha) return { href, text: p.sha.slice(0, 7) };
      break;
    case "oxplow/work_item": {
      if (workItemId(uri) === null) break;
      const whole = identityOf(uri) ?? uri;
      const id = workItemLabel(uri);
      return { href, text: mentionRef(id) === whole ? id : whole };
    }
  }
  return { href, text: title };
}

function collapseInternalLink(label: string, url: string): string | null {
  const uri = parseUri(url);
  if (uri !== null) {
    const p = uri.params;
    switch (typeOf(url)) {
      case "oxplow/file": {
        if (!p.path) return null;
        const line = /^(\d+)-\d+$/.exec(uri.view.lines ?? "")?.[1];
        // The authoring form: `path[@rev][:line]`, a git revision as its
        // bare token.
        const target = `${p.path}${p.rev ? `@${p.rev.startsWith("git:") ? p.rev.slice(4) : p.rev}` : ""}${line ? `:${line}` : ""}`;
        return label === target ? `[[${target}]]` : `[[${target}|${label}]]`;
      }
      case "oxplow/dir": {
        if (!p.path) return null;
        return label === p.path ? `[[dir:${p.path}]]` : `[[dir:${p.path}|${label}]]`;
      }
      case "oxplow/wiki": {
        if (!p.slug) return null;
        return label === p.slug ? `[[${p.slug}]]` : `[[${p.slug}|${label}]]`;
      }
      case "oxplow/commit": {
        const sha = p.sha;
        if (!sha) return null;
        // If the label is a prefix of the SHA (the bare `[[<sha>]]` form
        // renders as the short 7-char SHA), drop the label and emit the
        // bare wikilink. Otherwise preserve it.
        const labelIsHexPrefix = /^[0-9a-f]{7,40}$/i.test(label) && sha.toLowerCase().startsWith(label.toLowerCase());
        return labelIsHexPrefix ? `[[git:${sha}]]` : `[[git:${sha}|${label}]]`;
      }
      case "oxplow/work_item": {
        if (workItemId(url) === null) return null;
        // A bare `[[tsk42]]` (or the whole URI) renders with what was
        // written as its text (the title swap is a render-time overlay, not
        // stored), so a label equal to it collapses to the bare form.
        const id = workItemLabel(url);
        const whole = identityOf(url) ?? url;
        if (label === whole) return `[[${whole}]]`;
        return label === id ? `[[${id}]]` : `[[${id}|${label}]]`;
      }
      default:
        return null;
    }
  }
  if (url.startsWith(INVALID_LINK)) {
    // Preserve the author's original (invalid) target verbatim so the
    // round-trip through the editor doesn't rewrite or lose it — the
    // author still needs to see and fix it.
    const target = decodeURIComponent(url.slice(INVALID_LINK.length));
    if (!target) return null;
    return label === target ? `[[${target}]]` : `[[${target}|${label}]]`;
  }
  return null;
}

/**
 * Heuristic: does a wikilink target look like a repo file path rather
 * than a wiki note slug? File paths contain a slash or end in a recognizable
 * extension; bare slugs like `architecture` are routed to wiki navigation.
 */
/**
 * Heuristic: is the target a valid wiki-slug shape? Mirrors the backend
 * `crates/oxplow-domain/src/refs.rs::looks_like_slug` (kebab/snake, no
 * slash/dot/space, not a bare sha) so the renderer and the MCP link
 * checker agree on what's a real ref vs. a broken one — with the one
 * concession that a trailing `.md` is accepted (the renderer strips it),
 * keeping `[[some-note.md]]` a live link. `#13`, `@foo`, etc. fail here
 * and render broken.
 */
function looksLikeSlug(target: string): boolean {
  const bare = target.endsWith(".md") ? target.slice(0, -3) : target;
  if (!bare || bare.length > 80) return false;
  if (bare.includes("/") || bare.includes(".") || bare.includes(" ")) return false;
  if (/^[0-9a-f]{7,40}$/i.test(bare)) return false;
  return /^[A-Za-z0-9_-]+$/.test(bare);
}

/** The file URI a `[[path[@version][:line]]]` interior names. A `@disk` /
 *  `@local` version is the working tree (no `rev`); anything else is a git
 *  revision (the wiki's authoring syntax — `.context/vcs.md`). */
function fileTargetUri(target: string): string {
  let body = target;
  let rev: string | null = null;
  let line: number | null = null;
  const atIdx = body.indexOf("@");
  if (atIdx > 0) {
    const versionPart = body.slice(atIdx + 1);
    body = body.slice(0, atIdx);
    // The version may have a trailing `:<line>` anchor.
    let token = versionPart;
    const colonIdx = versionPart.lastIndexOf(":");
    if (colonIdx >= 0 && /^\d+$/.test(versionPart.slice(colonIdx + 1))) {
      token = versionPart.slice(0, colonIdx);
      line = Number(versionPart.slice(colonIdx + 1));
    }
    const lower = token.toLowerCase();
    if (lower !== "disk" && lower !== "local" && token) rev = revisionSlot(gitRevision(token));
  }
  if (line === null) {
    const m = body.match(/^(.+?):(\d+)$/);
    if (m) {
      body = m[1]!;
      line = Number(m[2]);
    }
  }
  return line === null ? fileUri(body, rev) : fileLinesUri(body, line, line, rev);
}

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
export function preprocessWikilinks(body: string, mentionRef: MentionRef = () => null): string {
  // Split out fenced code blocks (```...```) and protect them.
  const segments = body.split(/(```[\s\S]*?```)/g);
  return segments.map((seg, idx) => {
    if (idx % 2 === 1) return seg; // fenced block — leave alone
    return rewriteWikilinksOutsideInlineCode(seg, mentionRef);
  }).join("");
}

function rewriteWikilinksOutsideInlineCode(text: string, mentionRef: MentionRef): string {
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
        return `[${shortDisplay}](${linkTarget(commitUri(sha))})`;
      }
      // Directory form: explicit `dir:` prefix (mirrors `git:`).
      // Trailing slash on the path is tolerated and stripped.
      const dirTarget = target.startsWith("dir:") ? target.slice("dir:".length) : null;
      if (dirTarget !== null) {
        const dir = dirTarget.trim().replace(/\/+$/, "");
        if (dir) {
          // When no label was supplied, default to showing the bare
          // path (without the `dir:` prefix) so the rendered link
          // reads as a directory rather than a URL-shaped token.
          const dirDisplay = label ? display : dir;
          return `[${dirDisplay}](${linkTarget(dirUri(dir))})`;
        }
      }
      // A work item: its whole URI, or a bare id of the active list's
      // (`[[tsk42]]` with oxplow's tasks), as the backend's ref extractor
      // recognizes them for backlinks. The
      // rendered text defaults to the token; the item's title is swapped
      // in at render time (WikiLinkSpan + useWorkItemRef).
      const itemRef = workItemId(target) !== null ? (identityOf(target) ?? target) : mentionRef(target);
      if (itemRef) {
        return `[${display}](${linkTarget(itemRef)})`;
      }
      // An extension's ref kind, by its kind or its `wikilink:` prefix.
      const extensionRef = extensionWikilinkRef(target);
      if (extensionRef) {
        return `[${display}](${linkTarget(extensionRef)})`;
      }
      if (looksLikeFilePath(target)) {
        // The target may carry `@<version>` and `:<line>`; the file URI
        // names them (`rev`, `#lines`) and parseMarkdownLink reads them
        // back at click time.
        return `[${display}](${linkTarget(fileTargetUri(target))})`;
      }
      // Slug-shaped → wiki note. Anything else matches no known ref
      // shape (e.g. the GitHub `[[#13]]` form) → a broken link the
      // renderer shows as non-clickable. Encode the target so an odd
      // interior can't break the markdown link.
      if (looksLikeSlug(target)) {
        // Its URI, as every other ref: the editor keeps it as a link and
        // `postprocessWikilinks` gives the `[[slug]]` back.
        const slug = target.endsWith(".md") ? target.slice(0, -3) : target;
        return `[${display}](${linkTarget(wikiUri(slug))})`;
      }
      return `[${display}](${INVALID_LINK}${encodeURIComponent(target)})`;
    });
  }).join("");
}

/**
 * Inline anchor wrapper that owns the wiki-title swap. Lives as its
 * own component so `useWikiRef` (a hook) can be called per-link
 * without violating rules-of-hooks in the parent's render-prop.
 */
function WikiLinkSpan({
  anchorProps,
  handleLinkClick,
  items,
  iconKind,
  internalSlug,
  itemRef,
  extensionRef,
}: {
  anchorProps: React.AnchorHTMLAttributes<HTMLAnchorElement> & { node?: unknown };
  handleLinkClick: (event: React.MouseEvent<HTMLAnchorElement>) => void;
  items: MenuItem[];
  iconKind: string | null;
  internalSlug: string | null;
  /** A work item's ref. */
  itemRef: string | null;
  /** A ref of an extension's kind: titled from its kind's dataset. */
  extensionRef: string | null;
}) {
  const cm = useRowContextMenu(items);
  const { children, ...rest } = anchorProps;
  const { title: overrideText, brokenReason } = useRefLinkTitle({ internalSlug, itemRef, extensionRef }, flattenChildrenText(children));
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

/** What a link names that has a title: a wiki page, a work item, an
 *  extension's ref (`refLinkNames`). */
export interface RefLinkNames {
  internalSlug: string | null;
  itemRef: string | null;
  extensionRef: string | null;
}

/** The titled refs `parsed` names: none for a link with no title (a file,
 *  a directory, a commit, a web page). */
export function refLinkNames(parsed: ParsedLink): RefLinkNames {
  return {
    internalSlug: parsed.kind === "internal" ? parsed.slug : null,
    itemRef: parsed.kind === "work_item" ? parsed.ref : null,
    extensionRef: parsed.kind === "ref" ? parsed.ref : null,
  };
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
export function useRefLinkTitle(
  { internalSlug, itemRef, extensionRef }: RefLinkNames,
  text: string,
): { title: string | null; brokenReason: string | null } {
  const wiki = useWikiRef(internalSlug);
  const item = useWorkItemRef(itemRef);
  const extensionTitle = useExtensionRefTitle(extensionRef);
  const title =
    internalSlug && wiki.title && text === internalSlug
      ? wiki.title
      : itemRef && item.title && (text === workItemLabel(itemRef) || text === itemRef)
        ? item.title
        : extensionRef && extensionTitle && extensionWikilinkRef(text) === extensionRef
          ? extensionTitle
          : null;
  const brokenReason =
    internalSlug && wiki.status === "missing"
      ? `Broken link: wiki page “${internalSlug}” does not exist`
      : itemRef && item.status === "missing"
        ? `Broken link: work item ${workItemLabel(itemRef)} isn't on the active work list`
        : null;
  return { title, brokenReason };
}

/**
 * The ref a parsed link names, or `null` when it isn't a page at all
 * (external URLs, anchors, empty and broken refs). A click opens it
 * through `PageNavigation.openRef`, where its kind's page says (the
 * registry), the same for every kind under "open in a new tab" (tsk265).
 */
export function linkRef(parsed: ParsedLink): string | null {
  switch (parsed.kind) {
    case "file":
      // Bare wikilinks (no `@version`) coerce to WORKING; explicit versions
      // flow through as authored. Load-bearing: a wikilink that pinned
      // `@HEAD` must NOT be silently substituted with the working tree.
      return fileRef(parsed.path, parsed.version ?? WORKING).id;
    case "directory":
      return directoryRef(parsed.path).id;
    case "commit":
      return gitCommitRef(parsed.sha).id;
    case "work_item":
    case "ref":
      return parsed.ref;
    case "internal":
      return wikiPageRef(parsed.slug).id;
    default:
      return null;
  }
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
  /** Apply max-height + internal scroll instead of growing unbounded. */
  maxHeight?: number | string;
  /** Extra style overrides for the outer wrapper. */
  style?: CSSProperties;
  className?: string;
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
export function MarkdownView({ body, maxHeight, style, className }: MarkdownViewProps) {
  // Extensions' ref kinds change what a `[[…]]` names.
  const refKinds = useResources();
  // …and the active work list's ids do too.
  const profile = useWorkListProfile();
  const processedBody = useMemo(
    () => preprocessWikilinks(body, (id) => workItemRefOfMention(profile, id)),
    [body, refKinds, profile],
  );
  const ref = useRef<HTMLDivElement | null>(null);
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
    if (followLink(href, open, newTab)) event.preventDefault();
    // `open` is rebuilt per render from ctxNav.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ctxNav]);

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

  const wrapperClassName = ["oxplow-md", className].filter(Boolean).join(" ");

  return (
    <div ref={ref} className={wrapperClassName} style={wrapperStyle}>
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
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
            const items = linkMenu(href, open);
            const iconKind = parsedLinkIconKind(parsed);
            // For internal wiki links written as bare `[[slug]]` (no
            // `|label`), swap the rendered text from the slug to the
            // page title so readers see "Local Snapshots" not
            // `local-snapshots`. Author-supplied labels are preserved.
            const { internalSlug, itemRef, extensionRef } = refLinkNames(parsed);
            return (
              <WikiLinkSpan
                anchorProps={props}
                handleLinkClick={handleLinkClick}
                items={items}
                iconKind={iconKind}
                internalSlug={internalSlug}
                itemRef={itemRef}
                extensionRef={extensionRef}
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
