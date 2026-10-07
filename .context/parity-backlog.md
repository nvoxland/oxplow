# Tauri-migration parity backlog

All 11 originally-tracked items have landed. Keep this file as a
short history of what was caught + resolved during the parity sweep
in case any of these regress; delete when comfortable.

| # | Item                                                            | Status |
|---|-----------------------------------------------------------------|--------|
| 1 | Wire write_guard into PreToolUse                                | done   |
| 2 | Wire filing_enforcement into PreToolUse                         | done   |
| 3 | Wire stop_hook decideStopDirective into Stop                    | done   |
| 4 | Restart MCP / control plane on dev source change                | partial — see note below |
| 5 | Resume-tracker: capture session_id from hooks                   | done   |
| 6 | `read_task_options` MCP tool                                    | done   |
| 7 | `delegate_query` + `record_query_finding` MCP tools             | done; later removed |
| 8 | `reorder_tasks` MCP tool                                   | done; removed in P8.A10 (`oxplow.work_item.reorder` through `run_command`) |
| 9 | Wiki-note refs parser + backlinks (`find_wiki_pages_for_file`/`for_note`) | done   |
| 10 | Per-thread wiki-note attribution (`wiki_page_thread_update`)    | done   |
| 11 | Wiki-notes fs watcher                                           | done   |

## Item 4 caveat

True hot-reload of compiled-in MCP tools without restarting the Tauri
shell isn't practical with rmcp's tower service factory — Rust dylib
swap in-process needs a different scaffolding (`stabby`/`abi_stable`,
or moving oxplow-mcp behind a child-process boundary with stdio
transport that can be respawned). What landed instead: a
`POST /dev/ping` health-check endpoint on the control plane so dev
tooling can verify the server is up + the bearer token matches. For
real iteration, `bun run tauri:dev` rebuilds the Rust side on save.

## Useful follow-ups discovered during the sweep

These are smaller polish items, not parity-blockers, but worth filing
once the running daemon can reach `mcp__oxplow__create_task`
again on a fresh launch:

- The `write_guard` / `filing_enforcement` deny payloads now flow
  through `serde_json::to_value(deny)`. They serialize fine but the
  `Type` derives mean specta will eventually want to surface them in
  the TS bindings; not blocking, but a candidate for cleanup.
- `oxplow_mcp::OxplowMcp::new` is called per-MCP-session (rmcp
  factory). Each instance shares `Arc<Services>` so it's cheap, but
  if we ever want per-session state (cached lookups, etc.) the
  factory closure is the place.
