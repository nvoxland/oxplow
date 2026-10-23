/// A stream's open buffers as its language servers see them
/// (`.context/editor-and-monaco.md` "LSP bridge"): one mirror per stream,
/// the buffer store's `StreamServices` for it, started with the stream's
/// first buffer and stopped with its last. A buffer whose language has a
/// server is `didOpen`ed when it opens (or when its server appears), each
/// edit is a debounced `didChange` at the document's next version
/// (`DocumentSyncTracker`; Monaco's version ids reset on a model swap), a
/// save flushes and says `didSave`, and a close says `didClose` and
/// clears its markers. Whichever editor shows a file — or none — the
/// server sees the same text.
///
/// The mirror owns the stream's clients: diagnostics become markers on the
/// file's model, each language's status is heard by the editors showing
/// it (`status`, `subscribe`), and the server's `workspace/applyEdit` is
/// applied for the stream.
import type { Stream } from "../api.js";
import { languageForPath } from "../editor-language.js";
import { DocumentSyncTracker } from "../lsp-document-sync.js";
import type { Buffer, BufferStore, StreamServices } from "./buffers.js";

/** As much of `LspClient` as the mirror uses. */
export interface LspClientLike {
  notify(method: string, params?: unknown): void;
  request<T>(method: string, params?: unknown): Promise<T>;
  onDiagnostics(cb: (uri: string, diagnostics: unknown[]) => void): () => void;
  onStatus(cb: (message: string | null) => void): () => void;
  dispose(): void;
}

export interface MirrorDeps {
  hasServer(languageId: string): boolean;
  onServersChanged(cb: () => void): () => void;
  client(stream: string, languageId: string): LspClientLike;
  /** Put `diagnostics` on the model of `uri` (null: clear them). */
  markers(uri: string, diagnostics: unknown[] | null): void;
  uri(stream: Stream, path: string): string;
  /** Answer the server's `workspace/applyEdit` for the stream, saying how
   *  it went through `status`. Returns the stop. */
  applyEdit(stream: Stream, status: (message: string | null) => void): () => void;
  debounceMs?: number;
}

const mirrors = new Map<string, LspMirror>();

export class LspMirror implements StreamServices {
  /** The mirror of `stream`, while it has buffers. */
  static of(stream: string): LspMirror | undefined {
    return mirrors.get(stream);
  }

  private readonly clients = new Map<string, LspClientLike>();
  private readonly offs: (() => void)[] = [];
  /** Open documents, by path, with their language. */
  private readonly tracked = new Map<string, string>();
  private readonly tracker: DocumentSyncTracker;
  private readonly statuses = new Map<string, string | null>();
  private general: string | null = null;
  private readonly listeners = new Set<() => void>();

  constructor(
    private readonly stream: Stream,
    private readonly store: BufferStore,
    private readonly deps: MirrorDeps,
  ) {
    this.tracker = new DocumentSyncTracker((path, text, version) => {
      const lang = this.tracked.get(path);
      if (!lang) return;
      this.client(lang).notify("textDocument/didChange", {
        textDocument: { uri: deps.uri(this.stream, path), version },
        contentChanges: [{ text }],
      });
    }, deps.debounceMs ?? 200);
    mirrors.set(stream.id, this);
    this.offs.push(deps.onServersChanged(() => this.catchUp()));
    this.offs.push(deps.applyEdit(stream, (m) => this.report(m)));
  }

  /** The stream's client for `languageId` (made on first use). */
  client(languageId: string): LspClientLike {
    let c = this.clients.get(languageId);
    if (!c) {
      c = this.deps.client(this.stream.id, languageId);
      this.offs.push(c.onDiagnostics((uri, d) => this.deps.markers(uri, d)));
      this.offs.push(c.onStatus((m) => this.say(languageId, m)));
      this.clients.set(languageId, c);
    }
    return c;
  }

  /** Send `path`'s pending edit now: before a request about a position in
   *  it, so the server never answers against older text. */
  flush(path: string): void {
    this.tracker.flush(path);
  }

  /** What `languageId`'s server last said (an error, a crash), else the
   *  mirror's own last word (an applied edit); null when all is well. */
  status(languageId: string): string | null {
    return this.statuses.get(languageId) ?? this.general;
  }

  /** Say `message` to the stream's editors (an edit applied, an
   *  operation skipped); null clears it. */
  report(message: string | null): void {
    this.say(null, message);
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Drop `languageId`'s client (a server just installed): the next use
   *  makes another, and the open documents open on it again. */
  restart(languageId: string): void {
    this.clients.get(languageId)?.dispose();
    this.clients.delete(languageId);
    this.statuses.delete(languageId);
    for (const [path, lang] of [...this.tracked]) {
      if (lang !== languageId) continue;
      this.tracked.delete(path);
      this.tracker.close(path);
    }
    this.catchUp();
    this.notify();
  }

  opened(b: Buffer): void {
    const lang = languageForPath(b.path);
    if (this.tracked.has(b.path) || !this.deps.hasServer(lang)) return;
    this.tracked.set(b.path, lang);
    const version = this.tracker.open(b.path, b.draft);
    this.client(lang).notify("textDocument/didOpen", {
      textDocument: { uri: this.deps.uri(this.stream, b.path), languageId: lang, version, text: b.draft },
    });
  }

  changed(b: Buffer): void {
    if (this.tracked.has(b.path)) this.tracker.changed(b.path, b.draft);
  }

  saved(b: Buffer): void {
    const lang = this.tracked.get(b.path);
    if (!lang) return;
    this.tracker.flush(b.path);
    this.client(lang).notify("textDocument/didSave", {
      textDocument: { uri: this.deps.uri(this.stream, b.path) },
      text: b.saved,
    });
  }

  closed(path: string): void {
    const lang = this.tracked.get(path);
    if (!lang) return;
    this.tracked.delete(path);
    this.tracker.close(path);
    const uri = this.deps.uri(this.stream, path);
    this.client(lang).notify("textDocument/didClose", { textDocument: { uri } });
    this.deps.markers(uri, null);
  }

  dispose(): void {
    this.tracker.reset();
    this.offs.forEach((off) => off());
    for (const c of this.clients.values()) c.dispose();
    this.clients.clear();
    if (mirrors.get(this.stream.id) === this) mirrors.delete(this.stream.id);
  }

  /** Open on their servers the ready buffers not yet open there. */
  private catchUp(): void {
    for (const b of this.store.list(this.stream.id)) if (b.state === "ready") this.opened(b);
  }

  private say(languageId: string | null, message: string | null): void {
    if (languageId === null) this.general = message;
    else this.statuses.set(languageId, message);
    this.notify();
  }

  private notify(): void {
    for (const l of this.listeners) l();
  }
}
