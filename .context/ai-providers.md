# AI providers and roles

This doc covers oxplow's own access to models over their APIs: the
AI providers you configure, the roles that decide which model does what, the
`ai_*` functions sources and lenses call, and how calls are recorded.

> **Status (epic tsk275):** built: providers, keychain keys, roles, the
> client, call records (`oxplow-ai`, `oxplow-app/src/ai_service.rs`,
> `v_ai_call`), Settings → AI, and the `list_ai_roles` / `ai_decide` /
> `ai_summarize` MCP tools, inferred decisions, project role overrides,
> recorded computations (`AiCompute`, `v_ai_result`, P5.E1) and the
> `ai_*` Starlark builtins for collectors (P5.E2). **Not
> yet:** a models.dev catalog.
> Sections below say which parts are target design.

## Why

Terminal harnesses (Claude Code, Codex, opencode) remain the agents that do
the work. But oxplow itself benefits from calling models directly: to
classify an effort's risk, score a change, summarize a session, or answer a
typed yes/no question cheaply. This is **oxplow's own computation**, not
prompting a doing-agent, so it sits outside the no-automation invariant
([architecture.md](./architecture.md)).

What can be called, as of 2026-09:

- **Anthropic:** API keys, Bedrock or Vertex only. Subscription (Pro/Max)
  OAuth tokens may not be used in third-party tools.
- **OpenAI:** API keys; ChatGPT sign-in through the Codex App Server.
- **Google:** AI Studio or Vertex API keys; consumer OAuth is not allowed
  in third-party tools.
- **Open models and local servers:** OpenRouter, and OpenAI-compatible
  servers (Ollama, LM Studio, vLLM, LiteLLM).
- **TypeSafe Jev:** a *decision* model. It takes typed questions (choice,
  score, boolean) and returns calibrated probabilities in about 100 ms.

## Providers

- User-global, in `global_config_dir()/ai.yaml` (`OXPLOW_HOME` applies).
  `oxplow_ai::config::AiConfig` parses it (camelCase, unknown keys are
  errors) and `validate` runs before every `save`:

  ```yaml
  providers:
    - { id: or, kind: openrouter }
    - { id: local, kind: openai_compatible, baseUrl: http://localhost:11434/v1 }
    - { id: ts, kind: typesafe }
  roles:
    summarize: { provider: or, model: openai/gpt-5-mini }
    decide:    { provider: ts, model: jev-latest }
  ```

- Keys are stored in the **OS keychain** (`keyring` 4.2, service
  `net.voxland.oxplow`, account = provider id), never in the repo and never
  under the project dir. A provider with no key is called without auth
  (local servers). `SecretStore` is a trait; tests use `MemorySecrets`.
  Gotcha: on macOS an unsigned dev build is a "different app" after each
  rebuild, so the keychain may prompt again for access.
- **A key is bound to the URL it was saved for** (tsk346). The keychain
  entry is `{"key", "endpoint"}`, where `endpoint` is the provider's
  normalized `baseUrl` (empty for the kind's default). `ai.yaml` is a
  plain file an agent can edit, so a provider pointed at another host gets
  no key; the call fails with "re-save the AI provider in Settings → AI".
  Saving a provider (UI-only) rebinds a kept key to its current URL. A
  pre-binding bare key counts as bound to the default URL.
- **A provider's `kind:` is a declared model provider**: an
  `ai_provider` implementation an extension declares (`oxplow-foundation`:
  `anthropic`, `openai`, `openai_compatible`, `openrouter`, `typesafe`),
  registered in `ModelProviders` under its declared id. An unknown kind is
  refused by `AiService` naming the registered ones; a kind with no default
  URL (`openai_compatible`) needs a `baseUrl`, and every kind accepts one
  as an override. `AiConfig::validate` checks only the file's own shape
  (ids, role references); the kinds are the service's. Settings → AI offers
  the registered kinds (`AiSettings.kinds`: kind, title, default URL).
- Target: model catalog from models.dev (cached); manual entries allowed.

## Roles

Extensions and core refer to **roles, never to models**. Each role maps to
`{provider, model}`. Defaults are global (`ai.yaml`); a project overrides
any of them in `.oxplow/project.yaml`, which is committed, so the team
shares them:

```yaml
ai:
  roles:
    summarize: { provider: openrouter, model: openai/gpt-5-mini }
```

- Provider ids refer to each person's own `ai.yaml`; a role naming a
  provider someone hasn't set up shows "AI provider X isn't set up" for them.
- `OxplowConfig.ai_roles` holds them (validated against
  `oxplow_config::AI_ROLE_NAMES`, which a test keeps equal to `Role::ALL`).
- `AiService` gets an `OverridesSource` closure that reads the live config
  on every resolve, so edits and `reload_config_from_disk` apply with
  nothing to sync. Settings → AI shows such roles read-only
  ("Set by this project", hover says where to change them): its editor
  writes the global file, which the project value would override anyway.

| Role | Used for |
|---|---|
| `main` | general reasoning |
| `fast` | cheap, quick generation |
| `summarize` | summaries of sessions, efforts, changes |
| `embed` | embeddings |
| `decide` | typed questions with probabilities (Jev-shaped) |
| `review` | second-opinion review by a different model |

## Client

- **The interface** is `oxplow_ai::client::ModelProvider` (`kind`, `title`,
  `default_base_url`, `complete`, `decide`, `test`), called with a
  `ProviderInstance { id, base_url, key }` and a `CompleteRequest` /
  `DecideRequest`. `decide` defaults to a JSON prompt on `complete`
  (`decide_via_chat`); `test` to a one-word completion. `ModelProviders` is
  the registry (`Services.ai.providers()`), filled from the declarations by
  `ai_service::register_built_ins` at boot and on every capabilities
  refresh. Shared pieces stay in `oxplow-ai`: the `Http` POST helper and
  its error mapping, `bearer`, `parse_answers`, `strip_fences`.
- **The built-ins** live in `crates/oxplow-ai-providers` (`anthropic.rs`,
  `openai_compatible.rs`, `openrouter.rs`, `typesafe.rs`;
  `oxplow_ai_providers::built_in(entry, id, title, config)`), hand-rolled
  on the workspace's `reqwest` rather than `genai`: we need three small
  request shapes, and owning them keeps Jev's typed-question API
  first-class. `oxplow:openai-compatible` takes `config: { baseUrl? }` as
  its default URL (`openai` declares OpenAI's).
  - Anthropic Messages: `POST {base}/v1/messages`, `x-api-key`,
    `anthropic-version: 2023-06-01`.
  - OpenAI, OpenRouter, compatible servers: `POST {base}/chat/completions`,
    Bearer key, `response_format: json_object` when JSON is asked for.
  - TypeSafe Jev: `POST {base}/v1/systemone` with `{model, state,
    questions}`. A `noul` (yes/no) question has no criteria, a `choice`
    question's criteria are `{option: null}`, and a `score` question's are
    its ordered levels. OpenRouter serves Jev models at `/systemone` too.
- Two operations: `complete` (text, optionally JSON) and `decide` (typed
  questions: `noul` → probability, `choice` → choice + probabilities,
  `score` → level index + probabilities). Non-Jev models answer `decide`
  through a JSON prompt that describes the same answer shapes. TypeSafe
  only decides (its `complete` is an error) and its connection `test` asks
  a yes/no question; OpenRouter decides natively for a `jev` model.
- Errors: 401/403 → `Auth`, 429 → `RateLimited`, other non-2xx → `Http`
  with the start of the body.
- Answers are parsed from `serde_json::Value` by hand, not through the
  `Answer` derive: a dependency turns on serde_json's `arbitrary_precision`,
  which breaks numbers inside internally tagged enums.
- `AiService` (in `oxplow-app`, on `Services.ai`) is what callers use:
  role → provider + model + keychain key → call → record.
- **No budgets or cost tracking**, by the owner's decision (2026-09-26).
  Don't add them back without asking. Tokens are recorded.
- Every call, including failures, is a row in `ai_call` / `v_ai_call`
  (role, provider, model, caller, tokens, latency, ok, error), so AI
  usage is itself queryable in the [semantic layer](./semantic-layer.md).
- Tests use `oxplow_ai_fake::mock` (the dev-only `oxplow-ai-fake` crate,
  not a feature of `oxplow-ai`, so no build compiles it two ways —
  working-in-this-repo.md "Builds and `target/`"), a local axum server
  standing in for a provider. `Services::in_memory` gets
  `MemorySecrets` and a config dir under the test project's `.oxplow/`, so
  rpc/mcp tests never touch the real keychain or `ai.yaml`.
- `oxplow_app::ai_service` re-exports the `oxplow-ai` types the adapters
  need (`Role`, `ProviderConfig`, `Question`, …), so `oxplow-rpc`,
  `oxplow-tauri-ipc` and `oxplow-mcp` depend only on `oxplow-app`.

## Recorded computations (current, P5.E1)

`crates/oxplow-app/src/ai_compute.rs`, `Services.ai_compute`. A model
computation oxplow asks for is **recorded**: kept in `ai_result`
(`v_ai_result`, V118; the provider joined the key in V121) by
`UNIQUE (input_hash, provider, model, prompt_version)`,
where `input_hash = sha256(canonical JSON of { op, args })` (object keys
sorted, so argument order doesn't matter). Asking again for the same
thing — same input, same provider and model, same prompt — reads the recorded result:
`Recorded { value, cached: true, ai_call_id, input_tokens,
output_tokens }`, and **no call and no `ai_call` row** are made. A miss
calls, then records (a concurrent duplicate keeps the first). A failed
or unusable answer is never recorded. Tokens only; no cost.

| op | role | returns | prompt version |
|---|---|---|---|
| `classify(caller, text, labels)` | `decide` (a `choice` question) | `{ label, probabilities }` | `classify@1` |
| `score(caller, text, levels)` | `decide` (a `score` question, levels lowest first) | `{ level, score, probabilities }` | `score@1` |
| `summarize(caller, text, focus?)` | `summarize` (`summarize_system`) — the one summarize path: MCP `ai_summarize` asks it too | the summary | `summarize@1` |
| `extract(caller, instructions, text, schema)` | `main`, JSON mode; the schema is in the system prompt | JSON matching `schema` (checked with `InputValidator`; a mismatch is refused, naming the pointer) | `extract@1` |

Changing an op's prompt means bumping its version constant: old results
stay (for their version) and new ones are computed. The model is the
role's provider and model *now* (`AiService::binding_for`), so
reassigning a role computes afresh — to another model, or to another
provider serving the same model name (a local and a hosted `llama3.1`
are two computations, tsk560). `AiService::complete_as` / `decide_as` take a
`CallSite { caller, input_hash }`, so the computing call's `ai_call` row
carries the hash, and return its row id; plain `complete` / `decide`
record `input_hash` NULL.

## `ai_*` functions for sources (current, P5.E2)

An **entity collector** (a starlark derived source) calls `ai_classify(text,
labels)`, `ai_score(text, levels)`, `ai_summarize(text, focus = None)`
and `ai_extract(text, schema, instructions = "")` — Starlark builtins
(`crates/oxplow-script/src/ai.rs`) answered by the run's
`AiHost` in `Evaluator::extra` (the same pattern as a fact collector's
`TreeHost`). The host holds
a synchronous `dyn AiOracle`; the app's is `ai_compute::CollectorOracle`,
which blocks the script's worker thread on the runtime and asks
`AiCompute` as caller `collector:<owner>/<id>` — so every answer is a
recorded computation (the same question on the same text is one call,
ever). The time a script waits on the oracle is left out of its sandbox
`timeout` (`RunClock`, the in-flight call included;
`run_sandboxed_excluding`) but not out of its `ceiling` (10 min of wall
clock, model time included), so a per-row loop of calls can't run for
hours and hold up every scheduled source behind it. When the sandbox
gives up it stops the `RunClock`, and the detached worker's next `ai_*`
call is refused — no paid calls after a timeout. A run cut off keeps
what it recorded, so its next run gets further (tsk559). A **fact collector** (`facts:`,
run by the fact engine with a `TreeHost`) or a report parser runs without
an `AiHost`, and the builtins refuse: "`ai_classify` is available in
collectors only (a derived source) …" — a fact collector can't call a
model, and an entity collector has no `files()`. Lenses never call a model when they render; they read
what collectors recorded. The extension skill's example classifies each
turn's prompt (`turn_kind`).

## MCP (current)

- `list_ai_roles`: providers (with `keySet`, never keys) and every role's
  binding.
- `ai_decide`: typed questions (`noul` / `choice` / `score`) about some
  text, on the `decide` role unless another is named.
- `ai_summarize`: text through the `summarize` role, with an optional focus
  — a recorded computation (`AiCompute::summarize`, tsk571), so the same
  text and focus is one call.

Agents can't change providers, roles or keys: those IPC commands are
UI-only in the surface-parity manifest. Calls record caller `mcp:<tool>`.

## Inferred decisions (current)

The first built-in use of a role (`oxplow-app/src/inferred_decisions.rs`).

- The `effort.decisions` pump consumer (`effort_reactors.rs`) runs
  `infer_for_effort` for each `effort.finished` — logged once the
  effort's end snapshot is pinned — on its own loop, so a slow model
  never delays other consumers.
- Off until the `main` role has a model: `InferOutcome::Off`, no call.
- It digests the effort: task title; the thread's turns that overlap the
  effort's time window (`v_agent_turn` has no `effort_id`); its tool calls;
  and decisions already recorded, so they aren't repeated. The digest is
  capped at 40k characters, keeping the most recent turns.
- It is a recorded `AiCompute::extract` (`reply_schema()`:
  `{"decisions": [...]}`), so re-running an unchanged effort reads the
  recorded answer instead of calling again. At most 8 are kept; entries
  without a question or choice are skipped and a missing confidence
  becomes `low`.
- `SqliteReasoningStore::replace_inferred` swaps the effort's inferred rows
  in one transaction, so a re-run replaces rather than piles up. Open
  review lenses re-run on the `ModelsChanged` that commit makes
  (`v_decision`).
- Inferred rows are **never fed back to the agent** (the decisions block
  and the missing-decisions hint read `provenance = 'recorded'` only).
- Calls record caller `inferred-decisions` in `v_ai_call`.
- The review packet shows them as "Decisions Oxplow Noticed", separate from
  "Decisions Made".

## Settings → AI (current)

`apps/desktop/src/components/AiSection.tsx` (+ pure `aiSettingsModel.ts`),
a section of the Settings page. IPC: `ai_settings`, `save_ai_provider`,
`remove_ai_provider`, `set_ai_role` (announces `ConfigChanged`: a role's
binding is a row of the effective-config view, which refreshes on that
one signal), `test_ai_provider`.

- Providers: add or update by name (kind, base URL, key). A blank key keeps
  the saved one. Remove is refused while a role uses the provider, and
  deletes its key. Test makes one small call with a model you name (a
  `decide` question for Jev); it isn't recorded in `ai_call`.
- Roles: pick a provider and model per role; "Not assigned" clears it.
- Recent Calls: the last 7 days of `v_ai_call` by role and caller, read
  with `query_sql`, the same path lenses use.
