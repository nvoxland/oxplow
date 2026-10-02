# LSP subsystem

One shared, backend-owned LSP path. There is intentionally **no**
renderer-side language-server process management: the old raw
JSON-RPC bridge (`lsp_clients.rs`) was deleted because it never ran
the `initialize` handshake (servers rejected everything) and it
spawned a second server process per language. Don't reintroduce it.

## Ownership: `LspSessionManager` (`crates/oxplow-app/src/lsp_sessions.rs`)

One `LspProxy` (`crates/oxplow-lsp/src/proxy.rs` — spawn + JSON-RPC
framing/correlation only) per `(stream_id, language)`, spawned lazily
on first use and shared by the editor RPCs and the MCP tools. The
manager owns:

- **initialize** with real client capabilities (`client_capabilities()`
  — sync/diagnostics/hover/definition/references/completion/rename/
  codeAction/documentSymbol). Keep it in step with what the Monaco
  providers actually implement. `snippetSupport` is `true`: the
  completion mapping (`lsp-monaco-mapping.ts`) sets Monaco's
  `InsertAsSnippet` rule on items with `insertTextFormat: 2` — the two
  snippet syntaxes are the same TextMate dialect. The server's
  capabilities are stored on the session (`list_servers` exposes
  `completion_trigger_characters` from them).
- **Event pump** per session: server *notifications* re-emit as
  `LspSessionEvent::ServerNotification` on a manager broadcast
  (`subscribe()`); server→client *requests* are auto-answered
  (`workspace/configuration` → nulls, everything else → `null`) via
  `LspProxy::respond`. Without those answers rust-analyzer/gopls
  stall their own pipelines — this must stay in lockstep with the
  declared capabilities.
- **`workspace/applyEdit` is honored**, not auto-answered: the pump
  forwards it as `LspSessionEvent::ApplyEditRequest` (token + label +
  edit), the renderer applies it and answers via the
  `respond_lsp_apply_edit` RPC, and `respond_apply_edit` relays the
  verdict to the server. A timeout fallback (`APPLY_EDIT_TIMEOUT`,
  15s; immediate when the broadcast has no subscribers — headless
  MCP-only runs) answers `{applied:false}` so the server never stalls;
  late renderer answers are no-ops.
- **Document mirror**: `notify_session` intercepts
  `didOpen`/`didChange`/`didClose` (full-text sync) so a crashed or
  restarted server is respawned with every open buffer replayed as
  `didOpen`. Crash detection: pump sees `Closed`, removes the session
  (generation-checked so intentional restarts don't double-report) and
  emits `SessionStatus crashed`; the next request respawns.
- **Config resolution**: `.oxplow/project.yaml` `lsp.servers[]` first, then the
  Mason-installed registry (`InstalledServers`, carries package
  name/version). `NoConfig` errors are self-describing — they embed
  the curated Mason suggestion (`mason_suggestion`, mirrored by
  `apps/desktop/src/lspSuggestions.ts`; keep the two in sync) and both
  fix paths (the `lsp.install_server` command / yaml entry).

> **LSP id ↔ analysis `Language` bridge (tsk321).** The session `language`
> here is a free string (`languageId`), a separate namespace from the
> static-analysis `oxplow_code_metrics::Language` enum. The one documented
> bridge between them is `oxplow_code_metrics::language_from_lsp_id(id)` →
> `Option<Language>` (handles `typescriptreact`/`javascriptreact`, delegates
> the rest to `language_from_name`; an LSP language with no analysis grammar
> resolves to `None`). Use it whenever an LSP buffer needs tree-sitter
> analysis — don't re-derive the mapping. Part of the unified language-plugin
> epic (tsk320).

## Installer (`crates/oxplow-app/src/lsp_installer.rs`)

Wraps `crates/oxplow-lsp-installer/` (mason-org/mason-registry;
github-release sources only). Installs land in `.oxplow/lsp/<name>/`,
manifest at `.oxplow/lsp/installed.json` replays into the session
manager on boot. `remove()` reverses install (dir + manifest +
registrations). Install/remove emit `OxplowEvent::LspServersChanged`.

## Surface

- **RPCs** (`crates/oxplow-rpc/src/commands/lsp.rs`): `lsp_request`,
  `lsp_notify` (LSP payloads cross as **JSON strings** — specta emits a
  broken `Value` reference into bindings.ts otherwise), `list_lsp_servers`,
  `restart_lsp_server`, `list_installed_lsp_packages`,
  `respond_lsp_apply_edit`. Installing and removing a server are the
  `lsp.install_server` / `lsp.remove_server` commands (`commands/lsp.rs`,
  P8.A9: `External`, `Confirm::Always` — a person's call; each pushes
  `LspServersChanged`).
- **Events**: `lsp:event` carries `LspSessionEvent` (camelCase, tagged
  `kind`). Forwarded by the Tauri shell (`spawn_lsp_event_bridge`) and
  the daemon's `/events` WS (`lsp` frame). The renderer demux lives in
  `apps/desktop/src/lsp.ts` (`handleLspSessionEvent`), keyed by
  `(streamId, language)`.
- **Renderer**: `LspClient` facade + `lsp-servers-store.ts`
  (`hasLspServer` gating) + `lsp-document-sync.ts` (didChange versions
  + debounce). Workspace edits apply through
  `lsp-workspace-edit.ts`: open Monaco models via `pushEditOperations`
  (lands in the draft, undo intact), non-open files via
  `readFile`/`writeWorkspaceFile` read-modify-write (lands on disk);
  file create/rename/delete ops are still skipped + surfaced.
  `registerLspApplyEditHandler` in `lsp.ts` routes server-initiated
  applyEdits to the editor owning that stream (`useLspClients`
  registers per mounted editor; unclaimed requests answer
  `applied:false`). Editor wiring details:
  `.context/editor-and-monaco.md`.
- **Persisted diagnostics**: `lsp_diagnostics.rs` (spawned at boot)
  subscribes to the session broadcast and stores every
  `publishDiagnostics` in `lsp_diagnostic`, read as `v_diagnostic`
  (see `.context/semantic-layer.md`). Cleared at boot and per server on
  restart/crash/stop; a view of `v_diagnostic` re-runs on
  `ModelsChanged`. Debounced per stream, it logs **`code.diagnostics.changed@1 { stream, path, counts }`** once
  per changed file with its counts after the burst (a crash logs the
  files it cleared, at zero, and so does the boot clear, `clear_at_boot`;
  tsk571) — the durable record, on the event log.
- **Code intelligence (P5.C5)**: `oxplow_domain::code_intel::CodeIntelligence`
  — `definition`, `references`, `hover`, `document_symbols`,
  `workspace_symbols`, `call_hierarchy`, `diagnostics`, `rename` — over
  typed values (`Position { stream, path, line, col }`, `Location`,
  `Symbol { name, kind, container, location }`, `Call`, `Diagnostic`,
  `WorkspaceEdit`), 1-based and workspace-relative.
  **`LspProvider`** (`crates/oxplow-app/src/code_intel.rs`,
  `Services.code_intel`) is the one place that builds `textDocument/*`
  requests and maps the LSP's variants (`Location` / `LocationLink`,
  `DocumentSymbol` trees / `SymbolInformation`, `MarkupContent` /
  `MarkedString`, 0-based positions, URIs) to them. A file's language is
  the configured server whose `extensions` cover it — an installed
  (Mason) server's come from the language registry
  (`plugin::lsp_extensions`, set when it is registered; tsk556)
  (`LspSessionManager::language_for_path`); an uncovered file's error
  names the server to install (by the extension's language). Diagnostics
  are what the servers published (`lsp_diagnostic`), not a pull — and
  only a running server's: with none running for the file's language the
  answer is `CodeIntelError::NotRunning`, never `[]`, so an empty list
  always means clean (tsk558). Every
  request is bounded by `MachineEnv.lsp_request_timeout` (30 s in the
  app, 2 s in tests): a server that never answers is a
  `CodeIntelError::Failed` naming the language and method, not a hung
  caller (tsk557). `rename`
  returns the edits — from `documentChanges` when the server sends it,
  with its file creates / renames / deletes as `WorkspaceEdit.operations`
  in order, else from `changes` (tsk571) — and nothing applies them. A
  `Symbol` carries `location` (its name: `selectionRange`, what to jump
  to) and `extent` (the whole symbol: `range`). Subscribing to diagnostics is
  the event log's `code.diagnostics.changed`.
- **MCP**: `code_definition`, `code_references`, `code_hover`,
  `code_symbols`, `code_workspace_symbols`, `code_call_hierarchy`,
  `code_diagnostics` — thin calls into `Services.code_intel`, taking
  `{ stream_id, path, line, col }` (1-based) and answering typed JSON.
  They replaced the seven `lsp_*` tools that each built raw LSP requests.
  `lsp_list_servers` stays (provider-native); installing is the
  `lsp.install_server` command. The
  symbol tools work for **any** configured LSP language, not just the
  tree-sitter set; the call hierarchy runs `prepareCallHierarchy` then
  incoming/outgoing calls on the first item. Each MCP tool is registered
  in the surface-parity manifest (`crates/oxplow-surface-parity/src/lib.rs`).
  `workspace.symbol` and `textDocument.callHierarchy` are declared in
  `client_capabilities()`; keep the declared set + the pump's auto-answers
  in lockstep.
- **Symbols (P5.C6)**: the `symbols.collect` pump consumer
  (`crates/oxplow-app/src/symbol_collector.rs`, after `search.index`) turns
  each `snapshot.taken` into `symbol` rows (`v_symbol`): for each changed
  file a configured server covers, `document_symbols` restates the file
  (a deleted one drops out), refs `symbol:<path>/<name>@snap:<id>` —
  unique: a name path's later symbols in a file (an overload, a setter
  after its getter) are numbered in position order, `Widget::value~2` —
  with the name's position (`line`, `col`) and the whole extent
  (`start_*`..`end_*`, V123). It is
  bounded by config `symbolsMaxFilesPerSnapshot` (default 50; 0 turns it
  off) — the bound counts **attempts** (collected + failed), so a
  snapshot of files that all error or time out can't cost more than the
  bound in server time; the rest are recorded as over budget, and a
  failed file is counted in `files_failed`, never an error — and
  **never starts a server** (`LspSessionManager::is_running`): a collector
  doesn't run programs on a person's machine, so a file whose server
  isn't running is recorded as such. Each snapshot's coverage is a
  `symbol_capture` row (`v_symbol_capture`). Symbols are read from the
  file as it is when the event is handled, pinned to the snapshot that
  triggered it.
- **Conformance**: `code_intel_conformance::suite(provider, probe)` —
  definition and references come back as 1-based locations, a hover says
  something, document symbols name known kinds and nest through real
  containers, the call hierarchy answers, a rename proposes edits carrying
  the name, a reported diagnostic reads back and a lost (crashed) provider
  takes its reports with it and then says it isn't running. The language servers pass it over the fake
  python server, the crash being a real `die`.
- **RPC**: `lsp_request` / `lsp_notify` stay for Monaco's LSP bridge (a real
  LSP client); nothing else in the UI reads the servers yet.

## Pages (P6.E3)

The desktop reads code intelligence from the models (`apps/desktop/src/codeIntel.ts`):

- **Problems** (`page:problems`, `ProblemsPage.tsx`) — the stream's
  `v_diagnostic` rows by file (files with errors first, then warnings),
  a severity filter with counts; a problem opens its file at its line.
  It re-reads when `v_diagnostic` changes.
- **Symbols** (`page:symbols[?path=]`, `SymbolsPage.tsx`) — `v_symbol`
  nested under each symbol's `container` (`symbolTree`): one file's
  outline, or every file's symbols matching a name filter (a substring:
  `symbolsQuery` escapes LIKE's wildcards; the read follows typing after a
  200 ms pause). A symbol opens
  its file at its name's line.
- **`symbol:` refs** (a wikilink, an Ask, an answer) open the same way:
  `openSymbol` looks the ref up in `v_symbol`; a symbol that's gone (its
  file changed, or no server is running) is reported, not opened.

Both pages are in the launcher under Code. References and hover stay
live (the MCP `code_*` tools and the editor) — no model holds them.

## Testing

The python fake-server pattern is the way to test session behavior — a
real subprocess speaking framed JSON-RPC, no mocks. The shared one is
`crates/oxplow-app/src/lsp_fake.rs` (`lsp_fake::config(language,
extensions)`): initialize, the session pokes, `die`, `publish` (it
publishes what it's sent), and fixed answers to definition, references,
hover, document/workspace symbols, the call hierarchy and rename — what
`code_intel` tests against. The daemon has a WS
test asserting `LspSessionEvent` reaches the `lsp` frame
(`emit_event_for_tests` is the injection seam).
