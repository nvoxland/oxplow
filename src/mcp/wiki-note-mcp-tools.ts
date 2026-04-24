import { rmSync } from "node:fs";
import { join } from "node:path";
import type { ToolDef } from "./mcp-server.js";
import type { Stream } from "../persistence/stream-store.js";
import {
  computeFreshness,
  type FreshnessReport,
  type WikiNote,
  type WikiNoteStore,
} from "../persistence/wiki-note-store.js";
import { hashWorkspaceFile, notesDir, syncNoteFromDisk } from "../git/notes-watch.js";
import { readWorktreeHeadSha } from "../git/git.js";

export interface WikiNoteMcpDeps {
  resolveStream(streamId: string | undefined): Stream;
  wikiNoteStore: WikiNoteStore;
}

interface BaseArgs {
  streamId?: string;
}
interface SlugArgs extends BaseArgs {
  slug: string;
}

function projectDirFor(deps: WikiNoteMcpDeps, streamId: string | undefined): string {
  return deps.resolveStream(streamId).worktree_path;
}

function requireSlug(slug: unknown): string {
  if (typeof slug !== "string" || slug.length === 0) throw new Error("slug is required");
  if (slug.includes("/") || slug.includes("..")) throw new Error("slug must be a flat filename stem");
  return slug;
}

function summarizeNote(projectDir: string, note: WikiNote) {
  const currentHead = readWorktreeHeadSha(projectDir);
  const freshness: FreshnessReport = computeFreshness(
    { capturedHeadSha: note.captured_head_sha, capturedRefs: note.captured_refs },
    currentHead,
    (path) => hashWorkspaceFile(projectDir, path),
  );
  return {
    slug: note.slug,
    title: note.title,
    path: join(notesDir(projectDir), `${note.slug}.md`),
    created_at: note.created_at,
    updated_at: note.updated_at,
    freshness: freshness.status,
    head_advanced: freshness.headAdvanced,
    changed_refs: freshness.changedRefs,
    deleted_refs: freshness.deletedRefs,
    total_refs: freshness.totalRefs,
  };
}

export function buildWikiNoteMcpTools(deps: WikiNoteMcpDeps): ToolDef[] {
  const { wikiNoteStore } = deps;

  const STREAM_ID_SCHEMA = {
    type: "string",
    description: "Optional stream id. Defaults to the current stream.",
  } as const;

  return [
    {
      name: "oxplow__list_notes",
      description:
        "List wiki notes for this project with freshness badges. The note bodies live on disk at `.oxplow/notes/<slug>.md` — use the returned `path` with the Read tool to fetch content only for notes you actually want.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: STREAM_ID_SCHEMA,
          query: { type: "string", description: "Optional substring filter over note titles." },
          staleOnly: { type: "boolean", description: "If true, only return notes whose freshness is not 'fresh'." },
        },
      },
      handler: (args: BaseArgs & { query?: string; staleOnly?: boolean }) => {
        const projectDir = projectDirFor(deps, args.streamId);
        const rows = args.query ? wikiNoteStore.searchByTitle(args.query) : wikiNoteStore.list();
        const items = rows.map((n) => summarizeNote(projectDir, n));
        const filtered = args.staleOnly ? items.filter((i) => i.freshness !== "fresh") : items;
        return { notes: filtered };
      },
    },
    {
      name: "oxplow__get_note_metadata",
      description:
        "Return metadata + freshness detail for one note by slug. Does NOT return the body — read `.oxplow/notes/<slug>.md` directly with the Read tool for that.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: STREAM_ID_SCHEMA,
          slug: { type: "string", description: "Note slug (filename without .md)." },
        },
        required: ["slug"],
      },
      handler: (args: SlugArgs) => {
        const slug = requireSlug(args.slug);
        const projectDir = projectDirFor(deps, args.streamId);
        const note = wikiNoteStore.getBySlug(slug);
        if (!note) throw new Error(`note not found: ${slug}`);
        return summarizeNote(projectDir, note);
      },
    },
    {
      name: "oxplow__resync_note",
      description:
        "Force an immediate re-parse of `.oxplow/notes/<slug>.md` and re-baseline its freshness against current HEAD. Call this right after writing a note so its captured refs are pinned to the current workspace state without waiting for the watcher's debounce.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: STREAM_ID_SCHEMA,
          slug: { type: "string", description: "Note slug (filename without .md)." },
        },
        required: ["slug"],
      },
      handler: (args: SlugArgs) => {
        const slug = requireSlug(args.slug);
        const projectDir = projectDirFor(deps, args.streamId);
        syncNoteFromDisk(projectDir, wikiNoteStore, slug);
        const note = wikiNoteStore.getBySlug(slug);
        if (!note) return { slug, removed: true };
        return summarizeNote(projectDir, note);
      },
    },
    {
      name: "oxplow__search_notes",
      description:
        "Search wiki notes by title substring. Returns summaries; call Read on each `path` to pull bodies.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: STREAM_ID_SCHEMA,
          query: { type: "string", description: "Substring to match against note titles (case-insensitive)." },
        },
        required: ["query"],
      },
      handler: (args: BaseArgs & { query: string }) => {
        const projectDir = projectDirFor(deps, args.streamId);
        const rows = wikiNoteStore.searchByTitle(args.query);
        return { notes: rows.map((n) => summarizeNote(projectDir, n)) };
      },
    },
    {
      name: "oxplow__delete_note",
      description:
        "Delete a note: removes both the markdown file at `.oxplow/notes/<slug>.md` and the metadata row.",
      inputSchema: {
        type: "object",
        properties: {
          streamId: STREAM_ID_SCHEMA,
          slug: { type: "string", description: "Note slug to delete." },
        },
        required: ["slug"],
      },
      handler: (args: SlugArgs) => {
        const slug = requireSlug(args.slug);
        const projectDir = projectDirFor(deps, args.streamId);
        const filePath = join(notesDir(projectDir), `${slug}.md`);
        try {
          rmSync(filePath, { force: true });
        } catch {}
        wikiNoteStore.deleteBySlug(slug);
        return { slug, deleted: true };
      },
    },
  ];
}
