/**
 * What a markdown link means, with no React: `[[…]]` wikilinks rewritten
 * into links (`preprocessWikilinks`) and back (`postprocessWikilinks`,
 * `refLinkOf`), a link's target read (`parseMarkdownLink`) as the ref it
 * opens (`linkRef`), its title rule (`refLinkTitle`) and which URLs a
 * link may hold (`urlTransform`). The viewer (`MarkdownView`), the editor
 * and a component frame's markdown (`markdownTree`) share it.
 */
import { fileRef, directoryRef, gitCommitRef, wikiPageRef } from "../../tabs/pageRefs.js";
import { WORKING, gitRevision, revisionFromSlot, revisionSlot } from "../../revision.js";
import { taskId, taskLabel } from "../../taskRef.js";
import { dirUri, fileLinesUri, fileUri, identityOf, isOxplowScheme, linkTarget, commitUri, parseUri, typeOf, wikiUri, withUriView } from "../../uri/uri.js";
import { RENDERER_VIEW_KEY } from "../../pages/renderers.js";
import { extensionWikilinkRef, resourceInfo } from "../../resourceStore.js";

/** The prefix of a link the editor marked invalid: kept as written. */
const INVALID_LINK = "oxplow-invalid:";
const SAFE_PROTOCOL = /^(https?|ircs?|mailto|xmpp)$/i;

/**
 * What a link's URL may be: an oxplow URI (`oxplow:`, every resource's;
 * `.context/resources.md`) or an invalid-link mark as written; else a
 * relative URL or one of the safe protocols (http, https, irc, ircs,
 * mailto, xmpp) — react-markdown's default rule — and nothing (`""`)
 * otherwise, so no `javascript:` survives. The viewer's and a frame's.
 */
export function urlTransform(value: string): string {
  if (isOxplowScheme(value) || value.startsWith(INVALID_LINK)) return value;
  const colon = value.indexOf(":");
  const before = (c: string) => {
    const at = value.indexOf(c);
    return at !== -1 && colon > at;
  };
  if (colon === -1 || before("/") || before("?") || before("#") || SAFE_PROTOCOL.test(value.slice(0, colon))) return value;
  return "";
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
       *  was bare (`[[path]]`): it names the file as it is now, so the
       *  host opens `WORKING` (the working tree). Non-null carries the
       *  author's intent exactly. */
      version: import("../../revision.js").Revision | null;
      /** The filesystem the file is on (a relative link of a file drawn
       *  from another filesystem); none: the local one. */
      filesystem?: string | null;
      /** The page to draw it with (`#oxplow.renderer=`): a relative link
       *  to a file of the base's kind opens as the base is read. */
      renderer?: string;
    }
  | { kind: "directory"; path: string }
  | { kind: "commit"; sha: string }
  /** A task, by its URI. */
  | { kind: "task"; ref: string }
  /** A URI of an extension's resource (`oxplow:acme/pr?id=12`, P8.D7). */
  | { kind: "ref"; ref: string }
  /** A `[[…]]` whose target matches no known ref shape (e.g. the GitHub
   *  `[[#13]]` form). Rendered as a broken, non-clickable link. */
  | { kind: "broken"; reason: string };

const SHA_RE = /^[0-9a-f]{7,40}$/i;

/** Which task a loose id in text names, if any: the active list's
 *  ids, as it declares them (`taskRefOfMention`). */
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
    case "task":
      return "task";
    case "external":
      return "external-url";
    default:
      return null;
  }
}

/**
 * What a markdown link's `href` names. `base` is the URI of the file the
 * markdown came from (a `.md` drawn by the Markdown page): with one, a
 * relative href is a file in that file's folder, on its filesystem and
 * revision (`docs/guide.md`, `../img/x.png`, `/top.md` from the root), as a
 * repo's README means it. A link to a file of the base's kind (the same
 * extension) opens with the page the base's view names
 * (`#oxplow.renderer=`): a README read as a document links to `DEV.md` as
 * a document, where `src/main.rs` opens as what it is. With no base, a
 * relative `.md` link is a wiki slug, as the wiki's own pages mean it.
 */
export function parseMarkdownLink(rawHref: string, base: string | null = null): ParsedLink {
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
      case "oxplow/task":
        // Any list's item, by its URI.
        return taskId(rawHref) === null ? { kind: "empty" } : { kind: "task", ref: identityOf(rawHref) ?? rawHref };
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
      reason: `Broken link: “${target}” is not a recognized reference (use a task's id, [[some-slug]] for a wiki page)`,
    };
  }
  const relativeTo = base !== null && typeOf(base) === "oxplow/file" ? parseUri(base) : null;
  if (relativeTo?.params.path && !/^[a-z][a-z0-9+.-]*:/i.test(rawHref)) {
    const target = rawHref.split("#")[0]?.split("?")[0] ?? "";
    if (!target) return { kind: "empty" };
    const p = relativeTo.params;
    const path = relativeFilePath(p.path, safeDecode(target));
    const renderer = relativeTo.view[RENDERER_VIEW_KEY];
    return {
      kind: "file",
      path,
      version: p.rev ? revisionFromSlot(p.rev) : null,
      filesystem: p.filesystem ?? null,
      ...(renderer && extensionOf(path) === extensionOf(p.path) ? { renderer } : {}),
    };
  }
  let target = rawHref.replace(/^\.?\//, "");
  target = target.split("#")[0]?.split("?")[0] ?? "";
  if (target.endsWith(".md")) target = target.slice(0, -3);
  return target ? { kind: "internal", slug: target } : { kind: "empty" };
}

/** `path`'s extension, lowercased (`md`); empty with none. */
function extensionOf(path: string): string {
  const name = path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

function safeDecode(text: string): string {
  try {
    return decodeURIComponent(text);
  } catch {
    return text;
  }
}

/**
 * `href`, a relative path as a markdown file at `basePath` means it: from
 * the file's folder (`./` and `../` walked, never above the root), or from
 * the root when it starts with `/`.
 */
export function relativeFilePath(basePath: string, href: string): string {
  const folder = href.startsWith("/") ? [] : basePath.split("/").slice(0, -1);
  const out = folder.filter(Boolean);
  for (const part of href.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") out.pop();
    else out.push(part);
  }
  return out.join("/");
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
 * task's id when it is the active list's (`mentionRef`), else its
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
    case "oxplow/task": {
      if (taskId(uri) === null) break;
      const whole = identityOf(uri) ?? uri;
      const id = taskLabel(uri);
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
      case "oxplow/task": {
        if (taskId(url) === null) return null;
        // A bare `[[tsk42]]` (or the whole URI) renders with what was
        // written as its text (the title swap is a render-time overlay, not
        // stored), so a label equal to it collapses to the bare form.
        const id = taskLabel(url);
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
      // A task: its whole URI, or a bare id of the active list's
      // (`[[tsk42]]` with oxtask), as the backend's ref extractor
      // recognizes them for backlinks. The
      // rendered text defaults to the token; the item's title is swapped
      // in at render time (WikiLinkSpan + useTaskRef).
      const itemRef = taskId(target) !== null ? (identityOf(target) ?? target) : mentionRef(target);
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

/** What a link names that has a title: a wiki page, a task, an
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
    itemRef: parsed.kind === "task" ? parsed.ref : null,
    extensionRef: parsed.kind === "ref" ? parsed.ref : null,
  };
}

/** What a ref's lookups found: its wiki page's, its task's, its
 *  extension resource's title. */
export interface RefLookups {
  wiki: { title: string | null; status: "loading" | "found" | "missing" };
  item: { title: string | null; status: "loading" | "found" | "missing" };
  extensionTitle: string | null;
}

/** The title rule itself (`useRefLinkTitle`'s, and a component frame's
 *  `markdownTree`'s), over what the lookups found. */
export function refLinkTitle(
  { internalSlug, itemRef, extensionRef }: RefLinkNames,
  text: string,
  { wiki, item, extensionTitle }: RefLookups,
): { title: string | null; brokenReason: string | null } {
  const title =
    internalSlug && wiki.title && text === internalSlug
      ? wiki.title
      : itemRef && item.title && (text === taskLabel(itemRef) || text === itemRef)
        ? item.title
        : extensionRef && extensionTitle && extensionWikilinkRef(text) === extensionRef
          ? extensionTitle
          : null;
  const brokenReason =
    internalSlug && wiki.status === "missing"
      ? `Broken link: wiki page “${internalSlug}” does not exist`
      : itemRef && item.status === "missing"
        ? `Broken link: task ${taskLabel(itemRef)} isn't on the active task tracker`
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
      return withUriView(fileRef(parsed.path, parsed.version ?? WORKING, parsed.filesystem ?? null).id, { [RENDERER_VIEW_KEY]: parsed.renderer ?? null });
    case "directory":
      return directoryRef(parsed.path).id;
    case "commit":
      return gitCommitRef(parsed.sha).id;
    case "task":
    case "ref":
      return parsed.ref;
    case "internal":
      return wikiPageRef(parsed.slug).id;
    default:
      return null;
  }
}

