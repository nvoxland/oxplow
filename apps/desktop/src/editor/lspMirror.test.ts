import { expect, test } from "bun:test";

import { createBufferStore, type BufferIO, type ModelHandle } from "./buffers.js";
import { LspMirror, type LspClientLike, type MirrorDeps } from "./lspMirror.js";
import type { Stream } from "../api.js";

// One mirror per stream: the stream's open buffers, as the language
// servers see them — opened, changed (debounced), saved and closed as the
// buffers are, whichever editor (if any) shows them.

const stream = { id: "str1", worktree_path: "/w" } as Stream;

class Model implements ModelHandle {
  cbs: (() => void)[] = [];
  constructor(public text: string) {}
  getValue() {
    return this.text;
  }
  setValue(t: string) {
    this.text = t;
    this.cbs.forEach((c) => c());
  }
  onDidChangeContent(cb: () => void) {
    this.cbs.push(cb);
    return { dispose: () => {} };
  }
  dispose() {}
}

function fixture(servers: Set<string>) {
  const sent: string[] = [];
  const markers: [string, number | null][] = [];
  const statuses = new Map<string, (m: string | null) => void>();
  const diagnostics = new Map<string, (uri: string, d: unknown[]) => void>();
  let serversChanged = () => {};
  const models = new Map<string, Model>();
  const deps: MirrorDeps = {
    hasServer: (lang) => servers.has(lang),
    onServersChanged: (cb) => {
      serversChanged = cb;
      return () => {};
    },
    client: (_s, lang): LspClientLike => ({
      notify: (method, params) => {
        const p = params as { textDocument: { uri: string; version?: number } };
        sent.push(`${lang} ${method} ${p.textDocument.uri}${p.textDocument.version ? ` v${p.textDocument.version}` : ""}`);
      },
      request: async () => null,
      onDiagnostics: (cb) => {
        diagnostics.set(lang, cb);
        return () => {};
      },
      onStatus: (cb) => {
        statuses.set(lang, cb);
        return () => {};
      },
      dispose: () => sent.push(`${lang} dispose`),
    }),
    markers: (uri, d) => markers.push([uri, d === null ? null : d.length]),
    uri: (s, path) => `file://${s.worktree_path}/${path}`,
    applyEdit: () => () => {},
    debounceMs: 0,
  };
  const io: BufferIO = {
    read: async (_s, path) => (path === "gone.rs" ? null : { text: `text of ${path}` }),
    save: async () => {},
    model: async (_s, path, text) => {
      const m = new Model(text);
      models.set(path, m);
      return m;
    },
    services: (s, store) => new LspMirror(s, store, deps),
  };
  const store = createBufferStore(io);
  return { store, sent, markers, statuses, diagnostics, models, serversChanged: () => serversChanged() };
}

test("a buffer with a server opens on it; one without stays off it", async () => {
  const { store, sent } = fixture(new Set(["rust"]));
  await store.open(stream, "a.rs");
  await store.open(stream, "notes.txt");
  expect(sent).toEqual(["rust textDocument/didOpen file:///w/a.rs v1"]);
});

test("an edit is a didChange at the next version; a save flushes it and says didSave; a close says didClose and clears its markers", async () => {
  const { store, sent, models, markers } = fixture(new Set(["rust"]));
  await store.open(stream, "a.rs");
  models.get("a.rs")!.setValue("changed");
  await store.save("str1", "a.rs");
  store.retainOnly([]);
  expect(sent).toEqual([
    "rust textDocument/didOpen file:///w/a.rs v1",
    "rust textDocument/didChange file:///w/a.rs v2",
    "rust textDocument/didSave file:///w/a.rs",
    "rust textDocument/didClose file:///w/a.rs",
    "rust dispose",
  ]);
  expect(markers).toContainEqual(["file:///w/a.rs", null]);
});

test("a server that appears later opens the buffers already open", async () => {
  const servers = new Set<string>();
  const f = fixture(servers);
  await f.store.open(stream, "a.rs");
  expect(f.sent).toEqual([]);
  servers.add("rust");
  f.serversChanged();
  expect(f.sent).toEqual(["rust textDocument/didOpen file:///w/a.rs v1"]);
});

test("diagnostics become markers on the file's model", async () => {
  const f = fixture(new Set(["rust"]));
  await f.store.open(stream, "a.rs");
  f.diagnostics.get("rust")!("file:///w/a.rs", [{}, {}]);
  expect(f.markers).toContainEqual(["file:///w/a.rs", 2]);
});

test("the status of a language is the mirror's, heard by subscribers", async () => {
  const f = fixture(new Set(["rust"]));
  await f.store.open(stream, "a.rs");
  const m = LspMirror.of("str1")!;
  let heard = 0;
  m.subscribe(() => heard++);
  f.statuses.get("rust")!("LSP unavailable for rust");
  expect(m.status("rust")).toBe("LSP unavailable for rust");
  expect(heard).toBe(1);
});

