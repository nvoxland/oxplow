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
  fix paths (`lsp_install_server` / yaml entry).

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
  `restart_lsp_server`, `remove_lsp_package`, `install_lsp_package`,
  `list_installed_lsp_packages`, `respond_lsp_apply_edit`.
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
  restart/crash/stop. Debounced per stream, it emits `DiagnosticsChanged`
  and logs **`code.diagnostics.changed@1 { stream, path, counts }`** once
  per changed file with its counts after the burst (a crash logs the
  files it cleared, at zero) — the durable record, on the event log.
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
  the configured server whose `extensions` cover it
  (`LspSessionManager::language_for_path`); an uncovered file's error
  names the server to install (by the extension's language). Diagnostics
  are what the servers published (`lsp_diagnostic`), not a pull. `rename`
  returns the edits; nothing applies them. Subscribing to diagnostics is
  the event log's `code.diagnostics.changed`.
- **MCP**: `code_definition`, `code_references`, `code_hover`,
  `code_symbols`, `code_workspace_symbols`, `code_call_hierarchy`,
  `code_diagnostics` — thin calls into `Services.code_intel`, taking
  `{ stream_id, path, line, col }` (1-based) and answering typed JSON.
  They replaced the seven `lsp_*` tools that each built raw LSP requests.
  `lsp_list_servers` / `lsp_install_server` stay (provider-native). The
  symbol tools work for **any** configured LSP language, not just the
  tree-sitter set; the call hierarchy runs `prepareCallHierarchy` then
  incoming/outgoing calls on the first item. Each MCP tool is registered
  in the surface-parity manifest (`crates/oxplow-surface-parity/src/lib.rs`).
  `workspace.symbol` and `textDocument.callHierarchy` are declared in
  `client_capabilities()`; keep the declared set + the pump's auto-answers
  in lockstep.
- **RPC**: `lsp_request` / `lsp_notify` stay for Monaco's LSP bridge (a real
  LSP client); nothing else in the UI reads the servers yet.

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
