/**
 * What a markdown link means, with no React: `[[…]]` wikilinks rewritten
 * into links (`preprocessWikilinks`) and back (`postprocessWikilinks`,
 * `refLinkOf`), a link's target read (`parseMarkdownLink`) as the ref it
 * opens (`linkRef`), its title rule (`refLinkTitle`) and which URLs a
 * link may hold (`urlTransform`). The viewer (`MarkdownView`), the editor
 * and a component frame's markdown (`markdownTree`) share it.
 */
import { fileRef, directoryRef, gitCommitRef } from "../../tabs/pageRefs.js";
import { recordLabel, recordLinkText, recordOf, recordOfAnyBareId, recordOfBareId, recordOfStoreId } from "../../recordStores.js";
import { WORKING, gitRevision, revisionFromSlot, revisionSlot } from "../../revision.js";
import { dirUri, fileLinesUri, fileUri, identityOf, isOxplowScheme, linkTarget, commitUri, parseUri, typeOf, withUriView } from "../../uri/uri.js";
import { RENDERER_VIEW_KEY } from "../../pages/renderers.js";
import { extensionWikilinkRef, resourceInfo } from "../../resourceStore.js";
import type { RefTitle } from "../../refTitleCache.js";

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
  /** A URI of a resource other than the built-in kinds above: a row of a
   *  shared model (a task, a knowledge page), an extension's resource
   *  (`oxplow:acme/pr?id=12`). */
  | { kind: "ref"; ref: string }
  /** A `[[…]]` whose target matches no known ref shape (e.g. the GitHub
   *  `[[#13]]` form). Rendered as a broken, non-clickable link. */
  | { kind: "broken"; reason: string };

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
 * Classify a markdown link href. Pure — easy to test.
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
    case "file":
      return "file";
    case "directory":
      return "dir";
    case "commit":
      return "commit";
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
 * relative `.md` link is a record's id in a store whose ids look like it.
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
      default: {
        // URI-SWITCH: the registry is keyed by resource name (`v_resource`);
        // a row of a store of a shared model is one too.
        const resource = typeOf(rawHref);
        if (recordOf(rawHref) || (resource && resourceInfo(resource))) return { kind: "ref", ref: rawHref };
      }
    }
  }
  if (rawHref.startsWith(INVALID_LINK)) {
    const target = decodeURIComponent(rawHref.slice(INVALID_LINK.length));
    return {
      kind: "broken",
      reason: `Broken link: “${target}” is not a recognized reference (use a record's id, [[some-id]], or [[store:id]])`,
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
  const record = target ? recordOfAnyBareId(target) : null;
  return record ? { kind: "ref", ref: record } : { kind: "empty" };
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
 * Plain markdown links (http/https/relative slug with no scheme,
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
 * `postprocessWikilinks` collapses to the bare wikilink (a store's record by
 * its id when a bare id names it, else `store:id`, `recordLinkText`;
 * a file's `path[@rev][:line]`, a directory's path, a commit's short sha),
 * so it saves as `[[…]]` and shows its title (`useRefLinkTitle`).
 * A ref with no bare form keeps `title` as its label (`[title](uri)`).
 */
export function refLinkOf(uri: string, title: string): { href: string; text: string } {
  const href = linkTarget(uri);
  if (recordOf(uri)) return { href, text: recordLinkText(uri) };
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
    case "oxplow/commit":
      if (p.sha) return { href, text: p.sha.slice(0, 7) };
      break;
  }
  return { href, text: title };
}

function collapseInternalLink(label: string, url: string): string | null {
  if (recordOf(url)) {
    // A bare `[[tsk42]]` (or the whole URI) renders with what was written
    // as its text (the title swap is a render-time overlay, not stored), so
    // a label equal to it collapses to the bare form.
    const text = recordLinkText(url);
    const whole = identityOf(url) ?? url;
    return label === text || label === whole ? `[[${text}]]` : `[[${text}|${label}]]`;
  }
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
      case "oxplow/commit": {
        const sha = p.sha;
        if (!sha) return null;
        // If the label is a prefix of the SHA (the bare `[[<sha>]]` form
        // renders as the short 7-char SHA), drop the label and emit the
        // bare wikilink. Otherwise preserve it.
        const labelIsHexPrefix = /^[0-9a-f]{7,40}$/i.test(label) && sha.toLowerCase().startsWith(label.toLowerCase());
        return labelIsHexPrefix ? `[[git:${sha}]]` : `[[git:${sha}|${label}]]`;
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
 * than a store's record id? File paths contain a slash or end in a recognizable
 * extension; bare slugs like `architecture` are routed to a store's record.
 */
/** The file URI a `[[path[@version][:line]]]` interior names. A `@disk` /
 *  `@local` version is the working tree (no `rev`); anything else is a git
 *  revision (the wikilink authoring syntax — `.context/vcs.md`). */
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
  // Tail extension other than .md → file. .md → a record.
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
 * - `[[some-slug]]`               → the record a store's id pattern names; `[[store:id]]` one of any store's
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
      // A record of a store: its whole URI (`[[oxplow:…]]`). The rendered
      // text defaults to the token; the row's title is swapped in at render
      // time (`useRefLinkTitle`).
      if (recordOf(target)) {
        return `[${display}](${linkTarget(identityOf(target) ?? target)})`;
      }
      // A bare id of a store whose ids are loose (`tsk42`): its record,
      // ahead of a path or a slug.
      const loose = recordOfBareId(target, true);
      if (loose) return `[${display}](${linkTarget(loose)})`;
      // An extension's ref kind, by its kind or its `wikilink:` prefix.
      const extensionRef = extensionWikilinkRef(target);
      if (extensionRef) {
        return `[${display}](${linkTarget(extensionRef)})`;
      }
      // `[[store:id]]`: a record of any store of a shared model (the bare
      // `[[id]]` is the first store whose id pattern matches it, below).
      const colon = target.indexOf(":");
      if (colon > 0 && colon < target.length - 1) {
        const named = recordOfStoreId(target.slice(0, colon), target.slice(colon + 1));
        if (named) return `[${display}](${linkTarget(named)})`;
      }
      if (looksLikeFilePath(target)) {
        // The target may carry `@<version>` and `:<line>`; the file URI
        // names them (`rev`, `#lines`) and parseMarkdownLink reads them
        // back at click time.
        return `[${display}](${linkTarget(fileTargetUri(target))})`;
      }
      // A bare id another store's id pattern matches (a slug) →
      // that store's record, as its URI like every other ref: the editor
      // keeps it as a link and `postprocessWikilinks` gives the `[[id]]`
      // back. Anything else matches no known ref shape (e.g. the GitHub
      // `[[#13]]` form) → a broken link the renderer shows as non-clickable.
      // Encode the target so an odd interior can't break the markdown link.
      const bare = recordOfBareId(target.endsWith(".md") ? target.slice(0, -3) : target, false);
      if (bare) return `[${display}](${linkTarget(bare)})`;
      return `[${display}](${INVALID_LINK}${encodeURIComponent(target)})`;
    });
  }).join("");
}

/** The ref a parsed link names that has a title: none for a link with no
 *  title (a file, a directory, a commit, a web page). */
export function titledRef(parsed: ParsedLink): string | null {
  return parsed.kind === "ref" ? parsed.ref : null;
}

/** The title rule itself (`useRefLinkTitle`'s, and a component frame's
 *  `markdownTree`'s), over what the lookup found for `ref`: the title
 *  replaces the link's `text` when the link was written bare (its text is
 *  still the record's id or `store:id`, or an extension's `[[prefix:id]]`);
 *  a record the lookup finds no row for is broken. */
export function refLinkTitle(ref: string | null, text: string, found: RefTitle): { title: string | null; brokenReason: string | null } {
  if (!ref) return { title: null, brokenReason: null };
  const record = recordOf(ref);
  const bare = record ? [recordLabel(ref), recordLinkText(ref), `${record.store}:${record.id}`, identityOf(ref) ?? ref] : [];
  const written = bare.includes(text) || (!record && extensionWikilinkRef(text) === ref);
  const title = written && found.title ? found.title : null;
  const brokenReason =
    record && found.status === "missing"
      ? `Broken link: ${(resourceInfo(record.resource)?.label ?? "record").toLowerCase()} “${recordLinkText(ref)}” does not exist`
      : null;
  return { title, brokenReason };
}

/**
 * The ref a parsed link names, or `null` when it isn't a page at all
 * (external URLs, anchors, empty and broken refs). A click opens it
 * through `PageNavigation.openRef`, where its kind's page says (the
 * registry), the same for every kind under "open in a new tab".
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
    case "ref":
      return parsed.ref;
    default:
      return null;
  }
}

