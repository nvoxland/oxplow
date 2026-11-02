/// The working tree's open files (`.context/editor-and-monaco.md` "Buffers"):
/// one buffer per (stream, path) — the text saved on disk and the draft —
/// held for as long as a tab shows the file, whichever page draws it.
///
/// A buffer's draft lives in its Monaco model, made when the buffer opens
/// and disposed when it closes: typing and an LSP edit to a file not on
/// screen are both edits to the model, and so both the draft. Every
/// editor showing the file shares the model (and its undo history).
///
/// A stream's language services (`StreamServices`: the LSP mirror) start
/// with its first buffer and stop with its last, told of each open, edit,
/// save and close. The disk changing under a buffer replaces a clean one
/// and asks about a draft (`conflict`: Reload or Keep Mine).
import { useSyncExternalStore } from "react";

import type { Stream } from "../api.js";
import { isUnder, movedPath } from "../fileEvents.js";

export interface Buffer {
  stream: string;
  path: string;
  /** What the file holds on disk, as last read or saved. */
  saved: string;
  /** What the person has (the model's text). */
  draft: string;
  /** `missing`: the path is no file of the working tree; `unshown`: a file
   *  the editor cannot show (binary, too large, a folder; `error` says what
   *  it is) and so has no model; `error`: the read failed (`error` says why). */
  state: "loading" | "ready" | "missing" | "unshown" | "error";
  error: string | null;
  saving: boolean;
  /** The disk changed under unsaved edits: its text, until Reload or Keep
   *  Mine decides. */
  conflict: string | null;
}

/** It holds edits not yet saved. */
export function isDirty(b: Buffer | undefined | null): boolean {
  return !!b && b.state === "ready" && b.draft !== b.saved;
}

/** A buffer's model, as much of Monaco's `ITextModel` as the store needs. */
export interface ModelHandle {
  getValue(): string;
  setValue(text: string): void;
  onDidChangeContent(cb: () => void): { dispose(): void };
  dispose(): void;
}

/** What a stream's language services hear (the LSP mirror). */
export interface StreamServices {
  opened(b: Buffer): void;
  changed(b: Buffer): void;
  saved(b: Buffer): void;
  closed(path: string): void;
  dispose(): void;
}

/** What a read of a working-tree file gives: its text, or why it is not
 *  shown (`unshownReason`: binary, too large, a folder). */
export type FileRead = { text: string } | { unshown: string };

/** What the store needs from the world, injected so it tests alone. */
export interface BufferIO {
  /** The file in the working tree; null when it isn't there. */
  read(stream: string, path: string): Promise<FileRead | null>;
  /** Write it (`oxplow.file.save`), for the agent's `call` when one asked. */
  save(stream: string, path: string, content: string, call: CallRef | null): Promise<void>;
  /** A model holding `text` for the file. */
  model(stream: Stream, path: string, text: string): Promise<ModelHandle>;
  /** The stream's language services, started with its first buffer. */
  services?(stream: Stream, store: BufferStore): StreamServices;
  /** Follow the stream's worktree: a file changed on disk, renamed or
   *  deleted (by anyone). Returns the stop. */
  watch?(stream: string, on: { changed(path: string): void; renamed(from: string, to: string): void; deleted(path: string): void }): () => void;
}

/** The daemon's call a save answers (an agent's Save, written as it). */
export interface CallRef {
  client: string;
  id: string;
}

/** A closed dirty buffer's text, for Undo. */
export interface Stash {
  stream: Stream;
  path: string;
  saved: string;
  draft: string;
}

export interface BufferStore {
  /** Open `path`'s buffer (reading it once; again while it loads is the
   *  same read). */
  open(stream: Stream, path: string): Promise<Buffer>;
  get(stream: string, path: string): Buffer | undefined;
  /** The stream's buffers, in the order they opened. */
  list(stream: string): Buffer[];
  /** The model holding the buffer's draft; undefined until it's ready. */
  model(stream: string, path: string): ModelHandle | undefined;
  /** The stream, while it has buffers. */
  streamOf(stream: string): Stream | undefined;
  save(stream: string, path: string, call?: CallRef | null): Promise<void>;
  /** A dirty buffer's text, before its tab closes; null when clean. */
  stash(stream: string, path: string): Stash | null;
  restore(stash: Stash): Promise<void>;
  /** Close every buffer no tab shows now (`keep`: the ones that do). */
  retainOnly(keep: readonly { stream: string; path: string }[]): void;
  renamed(stream: string, from: string, to: string): void;
  deleted(stream: string, path: string): void;
  /** Read the file again after the disk changed under it. */
  diskChanged(stream: string, path: string): Promise<void>;
  /** A conflict resolved for the disk: the draft is what the disk has. */
  reload(stream: string, path: string): void;
  /** A conflict resolved for the draft: the disk's text is the saved one. */
  keepMine(stream: string, path: string): void;
  subscribe(listener: () => void): () => void;
  /** Bumped on every change (`useSyncExternalStore`'s snapshot). */
  version(): number;
}

const keyOf = (stream: string, path: string) => `${stream}\n${path}`;

interface Held {
  buffer: Buffer;
  model?: ModelHandle;
  off?: { dispose(): void };
}

interface StreamHeld {
  stream: Stream;
  services?: StreamServices;
  unwatch?: () => void;
}

export function createBufferStore(io: BufferIO): BufferStore {
  const held = new Map<string, Held>();
  const pending = new Map<string, Promise<Buffer>>();
  const streams = new Map<string, StreamHeld>();
  const listeners = new Set<() => void>();
  let version = 0;

  const notify = () => {
    version++;
    for (const l of listeners) l();
  };

  const set = (h: Held, patch: Partial<Buffer>) => {
    h.buffer = { ...h.buffer, ...patch };
    notify();
  };

  const streamHeld = (stream: Stream): StreamHeld => {
    let s = streams.get(stream.id);
    if (!s) {
      s = { stream };
      streams.set(stream.id, s);
      s.services = io.services?.(stream, store);
      s.unwatch = io.watch?.(stream.id, {
        changed: (path) => void store.diskChanged(stream.id, path),
        renamed: (from, to) => store.renamed(stream.id, from, to),
        deleted: (path) => store.deleted(stream.id, path),
      });
    } else {
      s.stream = stream;
    }
    return s;
  };

  const servicesOf = (stream: string) => streams.get(stream)?.services;

  /** Put `text` in the model as the person's, without it counting as an edit
   *  of theirs twice: the model's listener writes the draft. */
  const setDraft = (h: Held, text: string) => {
    if (h.model && h.model.getValue() !== text) h.model.setValue(text);
    else if (h.buffer.draft !== text) {
      set(h, { draft: text });
      servicesOf(h.buffer.stream)?.changed(h.buffer);
    }
  };

  const attach = async (stream: Stream, h: Held, draft: string) => {
    const model = await io.model(stream, h.buffer.path, draft);
    h.model = model;
    h.off = model.onDidChangeContent(() => {
      const text = model.getValue();
      if (text === h.buffer.draft) return;
      set(h, { draft: text });
      servicesOf(h.buffer.stream)?.changed(h.buffer);
    });
  };

  const close = (key: string) => {
    const h = held.get(key);
    if (!h) return;
    held.delete(key);
    h.off?.dispose();
    h.model?.dispose();
    const { stream, path } = h.buffer;
    const s = streams.get(stream);
    if (h.buffer.state === "ready") s?.services?.closed(path);
    if (s && ![...held.values()].some((o) => o.buffer.stream === stream)) {
      s.services?.dispose();
      s.unwatch?.();
      streams.delete(stream);
    }
    notify();
  };

  const load = async (stream: Stream, path: string, given?: { saved: string; draft: string }): Promise<Buffer> => {
    const key = keyOf(stream.id, path);
    const s = streamHeld(stream);
    const h: Held = {
      buffer: { stream: stream.id, path, saved: "", draft: "", state: "loading", error: null, saving: false, conflict: null },
    };
    held.set(key, h);
    notify();
    let read: FileRead | null;
    try {
      read = given ? { text: given.saved } : await io.read(stream.id, path);
    } catch (e) {
      set(h, { state: "error", error: e instanceof Error ? e.message : String(e) });
      return h.buffer;
    }
    if (held.get(key) !== h) return h.buffer;
    if (read === null) {
      set(h, { state: "missing" });
      return h.buffer;
    }
    if ("unshown" in read) {
      set(h, { state: "unshown", error: read.unshown });
      return h.buffer;
    }
    const text = read.text;
    const draft = given ? given.draft : text;
    await attach(stream, h, draft);
    if (held.get(key) !== h) {
      h.off?.dispose();
      h.model?.dispose();
      return h.buffer;
    }
    set(h, { saved: text, draft, state: "ready" });
    s.services?.opened(h.buffer);
    return h.buffer;
  };

  const store: BufferStore = {
    open(stream, path) {
      const key = keyOf(stream.id, path);
      const h = held.get(key);
      if (h && h.buffer.state !== "loading") {
        streamHeld(stream);
        return Promise.resolve(h.buffer);
      }
      const waiting = pending.get(key);
      if (waiting) return waiting;
      const p = load(stream, path).finally(() => pending.delete(key));
      pending.set(key, p);
      return p;
    },
    get: (stream, path) => held.get(keyOf(stream, path))?.buffer,
    list: (stream) => [...held.values()].map((h) => h.buffer).filter((b) => b.stream === stream),
    model: (stream, path) => held.get(keyOf(stream, path))?.model,
    streamOf: (stream) => streams.get(stream)?.stream,
    async save(stream, path, call = null) {
      const h = held.get(keyOf(stream, path));
      if (!h) throw new Error(`\`${path}\` isn't open`);
      if (h.buffer.state !== "ready") throw new Error(`\`${path}\` is still loading`);
      const content = h.buffer.draft;
      set(h, { saving: true });
      try {
        await io.save(stream, path, content, call);
      } catch (e) {
        set(h, { saving: false });
        throw e;
      }
      set(h, { saving: false, saved: content, conflict: null });
      servicesOf(stream)?.saved(h.buffer);
    },
    stash(stream, path) {
      const h = held.get(keyOf(stream, path));
      const s = streams.get(stream);
      if (!h || !s || !isDirty(h.buffer)) return null;
      return { stream: s.stream, path, saved: h.buffer.saved, draft: h.buffer.draft };
    },
    async restore(stash) {
      close(keyOf(stash.stream.id, stash.path));
      await load(stash.stream, stash.path, { saved: stash.saved, draft: stash.draft });
    },
    retainOnly(keep) {
      const wanted = new Set(keep.map((k) => keyOf(k.stream, k.path)));
      for (const key of [...held.keys()]) if (!wanted.has(key)) close(key);
    },
    renamed(stream, from, to) {
      for (const [key, h] of [...held.entries()]) {
        if (h.buffer.stream !== stream || !isUnder(h.buffer.path, from)) continue;
        const path = movedPath(h.buffer.path, from, to);
        const s = streams.get(stream);
        servicesOf(stream)?.closed(h.buffer.path);
        held.delete(key);
        h.buffer = { ...h.buffer, path };
        held.set(keyOf(stream, path), h);
        // Its model is the old file's; the moved one gets its own, the
        // draft carried over.
        h.off?.dispose();
        h.model?.dispose();
        h.model = undefined;
        if (s) {
          void attach(s.stream, h, h.buffer.draft).then(() => {
            s.services?.opened(h.buffer);
            notify();
          });
        }
      }
      notify();
    },
    deleted(stream, path) {
      for (const [key, h] of [...held.entries()]) {
        if (h.buffer.stream === stream && isUnder(h.buffer.path, path)) close(key);
      }
    },
    async diskChanged(stream, path) {
      const h = held.get(keyOf(stream, path));
      if (!h || h.buffer.state !== "ready" || h.buffer.saving) return;
      const read = await io.read(stream, path);
      // Gone, or no longer text: the buffer keeps what it has.
      if (read === null || "unshown" in read || held.get(keyOf(stream, path)) !== h) return;
      const disk = read.text;
      const b = h.buffer;
      if (disk === b.saved) return;
      if (disk === b.draft) {
        set(h, { saved: disk, conflict: null });
        return;
      }
      if (isDirty(b)) {
        set(h, { conflict: disk });
        return;
      }
      set(h, { saved: disk });
      setDraft(h, disk);
    },
    reload(stream, path) {
      const h = held.get(keyOf(stream, path));
      if (!h || h.buffer.conflict === null) return;
      const disk = h.buffer.conflict;
      set(h, { saved: disk, conflict: null });
      setDraft(h, disk);
    },
    keepMine(stream, path) {
      const h = held.get(keyOf(stream, path));
      if (!h || h.buffer.conflict === null) return;
      set(h, { saved: h.buffer.conflict, conflict: null });
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    version: () => version,
  };
  return store;
}

/** `path`'s buffer in `store`, re-rendering on each change to it. */
export function useBuffer(store: BufferStore, stream: string | null, path: string | null): Buffer | undefined {
  useSyncExternalStore(store.subscribe, store.version);
  return stream && path ? store.get(stream, path) : undefined;
}

/** The stream's buffers, re-rendering on each change. */
export function useStreamBuffers(store: BufferStore, stream: string | null): Buffer[] {
  useSyncExternalStore(store.subscribe, store.version);
  return stream ? store.list(stream) : [];
}
