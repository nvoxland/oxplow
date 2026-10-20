# AI providers and roles

This doc covers oxplow's own access to models over their APIs: the
AI providers you configure, the roles that decide which model does what, the
`ai_*` functions sources and lenses call, and how calls are recorded.

> **Status (epic tsk275):** built: providers, keychain keys, roles, the
> client, call records (`oxplow-ai`, `oxplow-app/src/ai_service.rs`,
> `v_ai_call`), Settings → AI, and the `v_ai_role` / `v_ai_provider` live models, inferred decisions, project role overrides,
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
- `OxplowConfig.ai_roles` holds them (a built-in name from
  `oxplow_config::AI_ROLE_NAMES`, which a test keeps equal to
  `BuiltinRole::ALL`, or a profile's `<extension>/<name>`, checked for
  shape only).
- `AiService` gets an `OverridesSource` closure that reads the live config
  on every resolve, so edits and `reload_config_from_disk` apply with
  nothing to sync. Settings → AI shows such roles read-only
  ("Set by this project", hover says where to change them): its editor
  writes the global file, which the project value would override anyway.

| Role | Used for |
|---|---|
| `main` | general reasoning: `extract` (a collector's `ai_extract`); the fallback of oxplow-bundled's `decisions` profile (inferred decisions) |
| `summarize` | summaries of sessions, efforts, changes (`summarize`) |
| `decide` | typed questions with probabilities, Jev-shaped (`classify`, `score`, `decide`) |

A role exists only once something asks it. A config naming a role oxplow
dropped (`fast`, `embed`, `review`: nothing asked them) fails to load,
saying so (`oxplow_ai::config::DROPPED_ROLES`; project.yaml's
`DROPPED_AI_ROLES`).

### Extension AI profiles

An extension declares its own model needs as **AI profiles**
(`ai_profiles:` in `extension.yaml`, [extensions.md](./extensions.md)):
an id, a description, a required `fallback` built-in role and optional
`features` (`text | tool_calling | streaming | image`). So
`oxplow_ai::config::Role` is open: `Builtin(BuiltinRole)` or
`Profile { extension, name }`, written as one string (`summarize`,
`effort-review/review`) wherever a person sees it — `ai.yaml`,
`project.yaml`'s `ai.roles`, Settings, `ai_call.role`, `ai_result.role`.

- **Naming.** Inside its own extension an author writes the bare id (a
  job request's `role`, a collector's `role=`); the host qualifies it
  with `Role::parse_in(extension, name)`. A bare name is a built-in or
  one of *this* extension's profiles: profiles aren't shared.
- **Resolution** (`AiConfig::resolve(role, profiles)`): a profile's own
  binding (project over global), else its fallback's. One no active
  extension declares resolves to nothing — "no active extension declares
  the AI profile `x/y`" — even if bound; unbound with an unbound fallback
  is "no model is assigned to the AI profile … or the `summarize` role it
  falls back to". `AiService` reads the active extensions' profiles
  through a `ProfilesSource` set after the catalog exists
  (`set_profiles`; the service is built first), live, like the overrides.
- **Features** are checked where a model's metadata is discovered anyway:
  a typed call (`prepare_typed`) on a profile whose bound model reports a
  needed feature `Unsupported` is refused before its key is read —
  "AI profile `x/y` needs tool calling, which `m` doesn't support" — and
  Settings shows the same as the row's `problem`. Unknown support passes.
- **Profiles are capability, tools are authority.** What a model may call
  stays on the job (`tools:`, admitted by the person when the extension is deployed, enforced per call by
  the bus); a profile only picks the model. Rebinding one never changes
  what a job may do or asks a person again. Declaring a profile
  authorizes nothing: it's reached only through the person's own provider.
- **A restriction is the person's side of that line.** A binding (global,
  project or a profile's own) may carry `restrict:` — a set of `tools`
  (any tool), `sql` (the `query_sql` tool), `streaming` (a streamed
  reply) and `images` (image input) it is forbidden
  (`oxplow_domain::ai::AiRestriction`, `RoleBinding.restrict`,
  `AiRoleOverride.restrict`):

  ```yaml
  ai:
    roles:
      summarize: { provider: openrouter, model: openai/gpt-5-mini, restrict: [tools, sql] }
  ```

  `prepare_typed` refuses a typed request needing what its binding
  forbids, before anything is sent and before the model's features are
  checked: "the `summarize` role's binding `or · m` is restricted: no
  tools" (`forbidden_by`). A job running on it fails with that error. An
  extension can't lift a restriction: profiles and jobs only choose the
  binding. A profile whose declared `features` its binding forbids shows
  the `problem` "AI profile `x/y` needs tool calling, which its binding
  `p · m` forbids" (`forbidden_feature`), and `extension check` warns the
  same. The files write `restrict` only when it isn't empty.
- **The cache needs nothing new.** A recorded computation is keyed by
  provider and model, the role is an attribute, so rebinding a profile
  computes afresh and old rows keep their provenance.
- `extension check` prints each profile and what serves it (bound, or the
  fallback); `extension test` runs on the throwaway's built-ins, so
  profiles fall back there.

## Client

- **The interface** is `oxplow_ai::client::ModelProvider` (`kind`, `title`,
  `default_base_url`, `discover`, `complete`, `complete_typed`, `decide`, `test`), called with a
  `ProviderInstance { id, base_url, key }` and a `CompleteRequest` /
  `DecideRequest`. `decide` defaults to a JSON prompt on `complete`
  (`decide_via_chat`); `test` to a one-word completion. `ModelProviders` is
  the registry (`Services.ai.providers()`), filled from the declarations by
  `ai_service::register_built_ins` at boot and on every capabilities
  refresh — each declared script under its extension's `ai` permission. Shared pieces
  stay in `oxplow-ai`: the `Http` POST helper (`post_with` for headers
  named at run time) and its error mapping, `parse_answers`,
  `decide_via_chat`.
- **Typed model contract (V1).** `capabilities::CapabilitySnapshotV1`
  reports per-model text, image, tool-calling and streaming support as
  `unknown`, `supported` or `unsupported`. Missing limits and token counts
  are unknown (`None`); a present zero is known zero. `discover` is metadata
  only and its default for existing providers reports unknown without
  making a completion or sending credentials. Capability names are not
  guessed from model names. Typed requests and responses use ordered
  `parts::MessageV1` / `PartV1` values and carry `schemaVersion: 1`.
  Image parts contain an opaque host-owned asset reference and media type,
  never a path, URL or inline bytes; resolving the asset belongs to the host
  transport. Tool definitions are separate from tool-call/result parts.
  The additive `complete_typed` default adapts only system messages followed
  by one user text message to `complete`, preserving their text. V1
  envelopes reject other schema versions during deserialization, and the
  adapter checks Rust-built requests before calling a provider. It rejects
  image and tool content, tool declarations and conversation shapes it
  cannot preserve before calling the provider. `UnsupportedFeature` carries
  whether support is unknown (images/tools on a text-only provider) or
  unsupported by the text adapter (conversation shapes it cannot preserve).
  Providers with native transport may override it. Because text-only replies
  carry neither a finish reason nor usage provenance, the adapter returns
  unknown for both; positive text-path token counts are preserved, with
  non-positive sentinel counts left unknown. Native typed providers may
  report observed or estimated usage. New typed usage has
  optional input/output counts and an explicit `unknown`, `observed` or
  `estimated` source; the text path's `Completion` and `Decision` token fields remain
  unchanged.
- **Discovery from extension hosts.** Collectors declare `ai.discover` and
  call `ai_capabilities(role = "main")` for the selected model or
  `ai_models(role = "main")` for its provider's declared catalog. This service
  needs no paid call and is usable in pure previews. Both resolve
  the role through `AiService` without reading the keychain; providers receive
  an instance with no key. They record no `ai_call` or AI usage. The host
  checks versions and model identities and rechecks run authority while
  waiting and before returning. Other hosts must supply an AI source through
  the shared router or report that discovery is unavailable.
  `ModelCatalogV1` distinguishes unknown from a known empty catalog and
  rejects unresolved or duplicate model identities. Existing providers
  return an unknown catalog. Scripted providers may declare `config.models`
  as V1 capability snapshots; discovery reads that data without executing
  `request`/`response`, checking credentials or contacting an endpoint.
  Missing models/features/limits remain unknown. Capability metadata is
  refreshed when the provider registry is refreshed, like its script.
- **Typed collector completion.** `ai_complete(request, role = "main")`
  is an `ai.compute` builtin and requires the same `ai` permission and live run
  checks as the text helpers. Its request is `{schemaVersion: 1, messages,
  tools?}`; the host binds the model from the role and refuses a supplied
  `model`. Unknown fields or versions fail instead of dropping semantics.
  `AiCompute::complete_typed` records and caches the full request and response
  under the role's provider/model binding. `AiService::complete_typed_as`
  refuses a request for a different model and known unsupported features
  before fetching a key. Optional usage counts and their provenance remain
  in `ai_call.response` and `ai_result.output`; the numeric aggregates
  include only observed usage on this path, using their zero
  sentinel for missing/non-observed values. Estimates never become observed
  usage, and known zero remains distinct from missing in the typed payload.
  Tool calls/results are ordered data and do not execute a tool automatically.
  Image input/output is currently refused at the app boundary because there
  is no authorized asset resolver; no reference is treated as a path or URL.
- **Streaming and cancelling.** `ModelProvider::stream_typed(instance,
  req, cancel, on_delta)` is `complete_typed` with the reply handed to
  `on_delta` as it arrives (`parts::StreamDeltaV1`: `text` as it comes, a
  `toolCall` once its arguments are whole), until `cancel` (the run's
  `oxplow_domain::run::Cancel`) is asked: then the call is dropped — its
  connection with it, which is what stops a provider — and the error is
  `AiError::Cancelled { partial }`, what had arrived. A stream silent for
  60 s fails `AiError::Timeout`. By default a provider "streams" its
  whole reply at once, at the end (`client::stream_whole`): every
  provider can, so nothing gates on the model's `streaming` capability.
  The transport is `Http::post_stream` → `SseStream` (server-sent events,
  `Accept: text/event-stream`; the status is read first and mapped as
  `post`'s; no total timeout, the idle one instead; events that arrive
  within 50 ms are one batch). `AiService::stream_typed_on` is
  `complete_typed_on` streamed, and records **every** exit — a cancelled
  call's `ai_call` row is `ok: false`, `error: cancelled`, its response
  the partial reply. `AiCompute::stream_typed` is the recorded
  computation: a recorded answer is handed over at once, whole, with no
  call; a cancelled or failed stream is never the recorded answer. A
  job's rounds use it ([extensions.md](./extensions.md) "Jobs");
  collectors don't stream.
- **Scripted providers.** Every model provider is a Starlark script an
  extension declares — oxplow's own too: `oxplow-foundation`'s
  `implementations:` name `providers/{anthropic,openai_compatible,openrouter,typesafe}.star`
  (`openai` is `openai_compatible.star` with OpenAI's `baseUrl`). There is
  no Rust built-in any more: `crates/oxplow-ai-providers` is only
  `scripted` (`Scripted: ModelProvider`). An `implementations:` entry whose
  `entry` isn't `oxplow:<name>` is a script — for `ai_provider` only —
  read and checked at load (`implementations::script_decl`: the file
  exists, defines `request` and `response`, and `config` is `{ baseUrl?,
  ops?, models? }`), carried as `Source::Script { entry, tree, script }`.
  - **The contract.** `request(x)` — `{ op: complete, model, system,
    prompt, json }` or `{ op: decide, model, state, questions }` (oxplow's
    question shapes) — returns `{ path, headers?, body }`, or `None` for a
    `decide` to ask as a chat. `response(x)` — `{ op, model, body }`, a 2xx
    reply's JSON — returns `{ text, usage? }`, `{ answers, usage? }`
    (oxplow's answer shapes, a noul as `probability`; parsed by
    `parse_answers`) or `{ error }`. `usage` is `{ input, output }`. Both
    are pure and sandboxed (`run_starlark_fn`, 5 s each).
    With `ops: [completeTyped]` (or alongside the text `complete`/`decide` ops), `request`
    receives `{op: completeTyped, model, request}` with the complete V1
    envelope; `response` receives `{op: completeTyped, model, body}` and
    returns a full V1 typed response, including usage provenance and finish
    reason. The same credential substitution and transport rules
    apply. Without that opt-in, typed completion uses the conservative
    text adapter. The shipped Anthropic and OpenAI-compatible mappings opt into
    `completeTyped` alongside text completion. They preserve conversation
    history, tool definitions and tool calls/results, optional observed usage
    (zero is retained) and finish categories. Image assets still require a host
    resolver and are refused. Anthropic requires initial text-only system
    messages, object tool inputs and results before user text. OpenAI chat
    requires result-only tool messages and refuses text after a tool call in an
    assistant message, since that transport separates content from calls.
    Unsupported response blocks fail instead of disappearing. Neither mapping
    executes a returned tool. OpenRouter/TypeSafe stay text-only (`complete`/`decide`).
    With `ops: [streamTyped]` the script defines a third function,
    `delta(x)` (checked at load): `request` gets `{ op: streamTyped,
    model, request }` and asks the vendor for a stream (its own `stream:
    true`); the host reads the reply's events and hands each batch to
    `delta` as `{ op, model, events: [{ event, data }], state }`, which
    returns `{ deltas, state }` — `deltas` are `StreamDeltaV1`s, `state`
    the script's own fold (text so far, tool-call argument fragments,
    usage, stop reason), carried to the next batch; at the end `response`
    gets `{ op, model, state }` and returns the whole V1 reply. The key is
    taken out of every event before `delta` sees it. Without
    `streamTyped` a stream is the whole reply at once.
    The shipped Anthropic, OpenAI and OpenAI-compatible mappings opt in
    (`ops: [complete, completeTyped, streamTyped]`); their `request` is the
    `completeTyped` body plus the vendor's stream switch, and `response`
    gives what `completeTyped` gives for the same content (same finish
    mapping and usage rules). Anthropic folds the Messages events:
    `message_start` (input usage with cache counts), `content_block_start`
    (a text block, or a tool_use block's id and name), `content_block_delta`
    (`text_delta` is a `text` delta at once; `input_json_delta` fragments
    are buffered per block), `content_block_stop` (a tool_use block's
    buffer is decoded into one `toolCall` delta), `message_delta` (stop
    reason, output usage) and `message_stop`; `ping` is ignored and an
    `error` event ends the call with its message. Chat completions folds
    `choices[0].delta.content` as text, `delta.tool_calls` fragments by
    `index` (the calls are handed over whole when the finish reason
    arrives), the final usage chunk (no choices; the body asks for it with
    `stream_options.include_usage`) and `[DONE]`; a refusal, audio or
    legacy `function_call` delta is refused as the whole reply refuses it.
    A stream that ends before the message is whole (no `message_stop`, no
    finish reason, a tool call still open) is an error, never a partial
    reply. OpenRouter and TypeSafe don't stream natively.
    Transport contracts: [OpenAI chat](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
    and [Anthropic tool calls](https://platform.claude.com/docs/en/agents-and-tools/tool-use/handle-tool-calls).
  - **The host does the call.** `path` is relative to the instance's base
    URL (its `baseUrl`, else the declaration's), so a call never leaves
    that host — no `network:` grant is needed. `{{key}}` in a header or the
    path is the instance's key, spliced by the host after `request`
    returns (`oxplow_domain::template`; the script never sees it); a header
    that needs a key the instance lacks is dropped (a local server). The
    host maps 401/403 → `Auth`, 429 → `RateLimited`, other non-2xx →
    `Http`.
  - **`ops`** lists what it answers natively: `complete` (the default)
    and `decide`, plus optional `completeTyped`. Without `decide`, a typed
    question is asked as a chat
    (`decide_via_chat`); one with only `decide` (TypeSafe) can't complete,
    and its connection `test` asks a yes/no question. A script that
    decides natively but not for every model (OpenRouter: only `jev`
    models, at `/systemone`) returns `None` for the rest.
  - **Permission** (decided 2026-10-08: shipped ones too, no exemption). An
    AI provider script is a program, so its extension must declare
    `permissions.ai` (an `ai_provider` implementation needs it; the loader
    refuses the extension otherwise), approved when the extension is
    deployed — oxplow-bundled's too, as shipped. There is no per-call
    gate: the deployment is immutable, so the script that runs is the
    script that was approved. **After an upgrade that changes a shipped provider
    script, the new deployment asks again only if it adds a permission.**
  - The shipped scripts speak:
    - Anthropic Messages: `POST {base}/v1/messages`, `x-api-key`,
      `anthropic-version: 2023-06-01`.
    - OpenAI, OpenRouter, compatible servers: `POST {base}/chat/completions`,
      Bearer key, `response_format: json_object` when JSON is asked for.
    - TypeSafe Jev: `POST {base}/v1/systemone` with `{model, state,
      questions}`. A `noul` (yes/no) question has no criteria, a `choice`
      question's criteria are `{option: null}`, and a `score` question's
      are its ordered levels. OpenRouter serves Jev models at `/systemone`
      too. Each has its test in `ai_service` (`the_<name>_script_…`).
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
- **No money budgets or cost tracking**, by the owner's decision
  (2026-09-26): a provider doesn't tell oxplow what a call cost. Don't add
  them back without asking. Tokens are recorded. A per-run limit on the
  *number* of model calls an extension may make was considered with
  scoped grants (2026-10-10) and left out for now: a grant says only
  whether a contribution may call a model, and each run's calls are
  counted (`v_extension_run.ai_calls`, `v_ai_call.run_id`).
- Every call, including failures, is a row in `ai_call` / `v_ai_call`
  (role, provider, model, caller, tokens, latency, ok, error, and
  `run_id` — the extension run that made it, V48), so AI usage is itself
  queryable in the [semantic layer](./semantic-layer.md). An `AiCompute`
  computation takes a `Caller { name, run }` (a bare name converts to
  one with no run); a collector's `CollectorOracle` names its run and
  counts each call on it (`extension_run.ai_calls`).
- **Every call keeps what it was asked and answered** (V41): the request
  as `AiService` hands it to the provider — `ModelRequest::Complete {
  system, prompt, json }` or `Decide { state, questions }`, its `body()`
  — and the response (`{ text }` or `{ answers }`; none for a failure),
  each stored whole (never cut: the request is a cache key) by its
  content hash in `event_content` under namespace `ai`
  (`ai_call_store::BODY_NAMESPACE`), the hashes on the row
  (`request_hash`, `response_hash`). `ai` is one of core's namespaces with
  no events; its bodies keep 30 days (`CORE_WINDOWS`) and the rows stay.
  A person reads one with RPC `read_ai_call { id, body: request |
  response }`, an agent with `read` on `ai_call:<id>#request` or
  `#response` — any call but one another agent's thread asked for
  (`oxplow_app::ai_calls::read`, capped at 64 KiB like an event body). What a provider adapter wraps around a request
  (`decide_via_chat`'s JSON instructions, an API's own envelope) is the
  adapter's, not part of it.
- Tests use `oxplow_ai_fake::mock` and `mock_sse` (a reply as timed
  server-sent event chunks, held open so a reader can be cancelled
  mid-stream) (the dev-only `oxplow-ai-fake` crate,
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
(`v_ai_result`) by `UNIQUE (input_hash, provider, model, request_hash)`,
where `input_hash = sha256(canonical JSON of { op, args })` (object keys
sorted, so argument order doesn't matter) and `request_hash` is the
content hash of the exact request the op sends (`ModelRequest::hash`,
the one its call keeps). Asking again for the same thing — same input,
same provider and model, same prompt — reads the recorded result:
`Recorded { value, cached: true, ai_call_id, input_tokens,
output_tokens }`, and **no call and no `ai_call` row** are made. A miss
calls, then records. Of two concurrent computations of one thing the
first recorded wins and **both return it** (`insert` returns what is
recorded for the key): one question reads one answer; the loser's call
stays in `v_ai_call`, paid for, with the same `input_hash`. A failed or
unusable answer is never recorded. Tokens only; no cost.

Every op takes the `role` it runs on (a built-in or a profile); the
column is what core's callers and the collector builtins default to.

| op | usual role | returns |
|---|---|---|
| `classify(caller, role, text, labels)` | `decide` (a `choice` question, `CLASSIFY_INSTRUCTIONS`) | `{ label, probabilities }` |
| `score(caller, role, text, levels)` | `decide` (a `score` question, levels lowest first, `SCORE_INSTRUCTIONS`) | `{ level, score, probabilities }` |
| `decide(caller, role, state, questions)` | `decide` (any named `noul` / `choice` / `score` questions) | answers by question name (a reply missing one is refused) |
| `summarize(caller, role, text, focus?)` | `summarize` (`summarize_system`) — the one summarize path | the summary |
| `extract(caller, role, instructions, text, schema)` | `main`, JSON mode; the schema is in the system prompt | JSON matching `schema` (checked with `InputValidator`; a mismatch is refused, naming the pointer) |

**The version follows the prompt.** There is no version constant to
bump: an op's request carries its whole prompt — its instructions, its
system text (`summarize_system`, a job's system message) — so editing any of it is another
`request_hash`, and the next ask computes afresh while the old results
stay under theirs. (A provider adapter's own wording isn't in the
request, so changing `decide_via_chat` doesn't.) The model is the
role's provider and model *now*: the computation binds the role once
(`AiService::bind`, under a catalog generation pin, since a profile is
an active extension's) and both keys the result by and calls on that
`Bound`, holding no pin across the model's reply — an activation never
waits on a model. So
reassigning a role computes afresh — to another model, or to another
provider serving the same model name (a local and a hosted `llama3.1`
are two computations, tsk560). `AiService::complete_on` / `decide_on` / `complete_typed_on` /
`stream_typed_on` take the `Bound` and a `CallSite { caller, input_hash }`, so the computing call's `ai_call` row
carries the hash, and return its row id; plain `complete` / `decide`
record `input_hash` NULL.

## `ai_*` functions for sources (current, P5.E2)

An **entity collector** (a starlark derived source) calls `ai_classify(text,
labels)`, `ai_score(text, levels)`, `ai_summarize(text, focus = None)`
and `ai_extract(text, schema, instructions = "")`, each with a keyword
`role=` (defaults `decide`, `decide`, `summarize`, `main`; or one of its
extension's AI profiles by id, qualified with the run's extension) — Starlark builtins
(`crates/oxplow-script/src/ai.rs`) answered by the run's
`AiHost`, one of the services the run was given (`runtime::Hosts`,
[extensions.md](./extensions.md) "Services"). The host holds
a synchronous `dyn AiOracle`; the app's is `ai_compute::CollectorOracle`,
which blocks the script's worker thread on the runtime and asks
`AiCompute` as caller `collector:<owner>/<id>` — so every answer is a
recorded computation (the same question on the same text is one call,
ever). Only a script whose extension declares `permissions.ai` gets it: one that
names an `ai_*` builtin is refused otherwise, like an exec program needing
`process` ([semantic-layer.md](./semantic-layer.md) "Sandbox"). The script runs in a script worker
([collection.md](./collection.md) "Containing scripts"); each `ai_*` call
crosses to the host, which asks the oracle. The time it waits is left out
of the run's `timeout` (`RunClock`, the in-flight call included) but not
out of its `ceiling` (10 min of wall clock, model time included), so a
per-row loop of calls can't run for hours and hold up every scheduled
source behind it. Past either the worker is killed: no paid calls after
a timeout. A run cut off keeps
what it recorded, so its next run gets further (tsk559). Who gets a model is the
services router's ([extensions.md](./extensions.md) "Services"): an
entity collector whose script names an `ai_*` builtin implies
`ai.compute`; a **fact collector** gets one when it declares
`needs: [ai.compute]` and its extension declares `permissions.ai` (otherwise
it is refused before it runs); a script that composes — a command's, an effect's, a
policy's, a review's dry run — runs pure and is refused before it runs:
"`ai_summarize()`: a pure run can't use `ai.compute` (ask a model): only
a run that may call out can". A run not given a model (a report parser)
is told the need: "`ai_classify()` needs `ai.compute` in this script's
`needs`". Lenses never call a model when they render; they read
what collectors recorded. The extension skill's example classifies each
turn's prompt (`turn_kind`).

## MCP (current)

- `v_ai_provider` (with `key_set`, never keys) and `v_ai_role` (every role's
  binding, what a call would use now, and whether its provider has a key),
  read live: `ai.yaml`, `project.yaml` and the keychain change with no hook.
- `oxplow_ai_decide`: typed questions (`noul` / `choice` / `score`) about some
  text, on the `decide` role (an agent can't pick a costlier one) — a
  recorded computation (`AiCompute::decide`), returning `{ answers,
  cached }`.
- `oxplow_ai_summarize`: text through the `summarize` role, with an optional focus
  — a recorded computation (`AiCompute::summarize`), so the same text and
  focus is one call.
- What a call was asked or answered is `read` of its `ai_call` ref (the
  caller's own calls).

Each is paid with the person's key, so neither is a read-only tool (no
`read_only_hint`: a harness doesn't auto-approve it), each takes at most
`AI_TEXT_MAX` (100,000) characters of text, and each records as the
asking thread (caller `thread:<id>`, from the session's bearer); a
connection with no thread identity is refused. Agents can't change
providers, roles or keys: those IPC commands are UI-only in the
surface-parity manifest.

## Inferred reasoning: decisions and claims (current)

When an effort finishes, one model call reads what the agent did and
proposes both the decisions it made without recording them and the claims
its answers made about its work. The policy — when to ask, the prompt,
which model, how the reply is read — is oxplow-bundled's, built only from
what any extension has; core keeps the storage and the review. What an
agent records itself is the other source: decisions with
`effort.record_decision`, claims it can back in `effort.report`'s
`claims`. Both land in the same tables, told apart by `provenance`
(`recorded` | `inferred`), and the review treats a claim alike whatever
its source; with no model bound, the recorded ones are all there is.

- **oxplow-bundled** declares the AI profile `reasoning` (fallback
  `main`), the job `reasoning` (`ai.complete_typed`, `needs: [ai.compute]`,
  no tools: the model reads only what the prompt gives it), the command
  `oxplow.effort.infer_reasoning { effort, again? }`
  (`handlers/infer_reasoning.star`) and two effects.
- **`infer-reasoning`** (on `effort.finished`) asks
  `scope("ai.discover", { role: "reasoning" })`; with no model bound to
  the profile or to Main it *skips*, saying to bind one in Settings → AI —
  in Delivery, counted against no health, nothing queued to fail.
  Otherwise it composes `oxplow.effort.infer_reasoning`.
- **The command** digests the effort: the work item's title; the
  thread's turns overlapping the effort's window (`v_agent_turn` has no
  `effort_id`), each as the human prompt and the agent's final answer
  (`v_agent_turn.answer`); its tool calls; the decisions it recorded and the
  claims it stated, so they aren't repeated. A prompt is clipped to its first 2,000
  characters, an answer to its last 2,000 (an answer states its outcome at
  the end), a tool's detail to 200, tool calls take at most a quarter of
  the 40,000-character digest, and the newest are kept. Nothing to read
  asks nothing. It enqueues the job with intent key
  `reasoning:<effort ref>` (a redelivery is the same job); `again: true`
  (the lens's **Infer Again**) keys a new one, `…@<n>`. The system
  message asks for JSON only
  (`{"decisions": [...], "claims": [...]}`): a typed request has no JSON
  mode. A claim is what an answer states about the outcome of the work —
  tests pass, behavior unchanged, a case handled — in the agent's words,
  with an `evidence_ref` only when the answer itself named a run or test;
  the model is told to report only claims actually made, never to infer
  one.
- The job's model call is a recorded computation keyed by its request,
  so asking again about unchanged activity on the same binding reads the
  recorded answer; rebinding the profile asks the new model.
- **`store-reasoning`** (on `contribution.job.completed`, `where: {
  contribution: job:oxplow-bundled/reasoning }`) reads the job, drops a
  code fence, and skips — saying so — a reply that isn't a JSON object
  with a `decisions` or a `claims` list (Starlark's `json.decode` has no
  fallback, so the check is the object's braces; malformed JSON inside
  them fails the run). Entries without a question and choice (decisions)
  or a statement (claims) are dropped, strings are clipped to the
  commands' bounds, an unknown decision confidence is left out (it
  becomes `low`), an unknown claim `kind` becomes `other`, and at most 8
  of each are kept. It calls core's `oxplow.effort.replace_inferred_decisions`
  and `oxplow.effort.replace_inferred_claims`, both every time, even with
  an empty or missing list (clearing stale ones).
- **Core** keeps `oxplow.effort.replace_inferred_decisions`
  (`commands/reasoning.rs`, the only writer of provenance `inferred`;
  thread and work item are the effort's; recorded, confirmed and
  dismissed rows stay; logs `effort.decisions_inferred@1 { effort,
  work_item?, count }`) and `oxplow.effort.replace_inferred_claims` (the
  only writer of `inferred` claims; it replaces the effort's inferred
  claims whose `review` is NULL, so a verified or dismissed claim and the
  agent's `recorded` ones stay), the review commands
  (decisions: confirm, dismiss, reopen; claims: `verify_claim`,
  `dismiss_claim`, `reopen_claim`, all a person's), and the rule that
  inferred rows are **never fed back to the agent** (the decisions block
  and the missing-decisions hint read `provenance = 'recorded'` only).
- A claim's `review` (`v_claim.review`) is NULL (unreviewed), `verified`
  or `dismissed`. oxplow-bundled's `claim_status` counts a verified claim
  as backed and leaves a dismissed one out entirely (neither backed nor
  unbacked), so the verdict, `review.accept`, `review.request_changes`
  and the verify-unchecked effect ignore it.
- Calls record role `oxplow-bundled/reasoning` in
  `v_ai_call`. The review packet shows the decision rows as "Decisions
  Oxplow Noticed", separate from "Decisions Made", and the claim rows in
  "Unverified Claims" with their source (stated or read) and review state.
- Disabling oxplow-bundled, removing its permissions or replacing them with
  another extension's (an effect that calls the same commands) needs no
  core change. Pinned end to end in `oxplow-sdk/tests/examples.rs`.

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
- Roles: the built-ins (their text is `BuiltinRole::description`), then
  each active extension's AI profiles (`RoleStatus.extension`,
  `fallback`, `effective`, `problem`), then bindings for profiles nothing
  declares (a problem, so they can be cleared). Pick a provider and model
  per role; "Not assigned" clears it, and an unbound profile shows "Falls
  back to summarize (provider · model)". With a provider chosen,
  **Allows** Tools / SQL / Streaming / Images checkboxes (all on by
  default) set the binding's `restrict` with the same Save, Escape putting
  them back; a restricted row says "Restricted: no tools, no SQL"
  (`aiSettingsModel.ts` `ALLOWED`, `allowedOf`, `restrictionsOf`).
- Recent Calls: the last 7 days of `v_ai_call` by role and caller, read
  with `query_sql`, the same path lenses use.

## Host transport and live authority

The shared `Http` client refuses redirects; a provider request cannot carry
credentials or content to a redirected destination. Transport errors strip
URL details, including query values used by key placeholders. Scripted AI
mappings see placeholders, never the provider key: the host substitutes only
in transport paths/headers. Routing/proxy header overrides are refused, and
exact credential echoes in JSON responses/error bodies are redacted before
mapping scripts see them. A last live-owner check runs after request mapping
and immediately before HTTP. The check is pinned to the loaded version;
a redeployed script does not lend its authority to the old mapping.
Collector AI calls recheck their parent run before each request, poll live
authority while waiting and discard responses after cancellation or when the extension is disabled.
Dropping a request future cannot retract a request already sent.

### AI runtime reader ownership

Boot adopts the shared extension activation boundary into AiService. Completion,
decision, connection checks and discovery pin before provider lookup; recorded
computations retain that scope through cache lookup, invocation and result
attribution. Nested calls reuse the reader. Settings ingress uses the async
pinned listing; the synchronous helper remains for standalone callers.

Each computation captures one key-free effective AI configuration, shared by
nested calls on that service. A role change while awaiting a provider therefore
cannot label the returned result with a different model's cache identity.
Credentials and authority are still fetched/checked by their host and are never
part of the captured configuration. Script worker oracles re-enter with their
RunContext pin before discovery/computation; this avoids a second fair-lock
reader deadlocking behind a waiting generation writer. Typed role binding is
resolved inside that same scope.

Typed AI jobs accept a strict role-based envelope (one MiB, depth 64), checked
before recursive JSON serialization or typed deserialization. The host binds
the model when dispatching, and records the full typed result under the same
captured configuration as the cache key. Anthropic observed input counts include
uncached input plus reported cache reads/creation; missing input remains unknown.
See [Anthropic token accounting](https://platform.claude.com/docs/en/build-with-claude/prompt-caching).

## The fixture model

`oxplow_ai::fixture::FixtureModel` (kind `fixture`, registered with
`ModelProviders::register_host`, never declared) answers in-process from a
list of replies, each a string or `FixtureReply { text, tool_calls,
chunk_delay, when }` (in a fixture: `{ text?, toolCalls?: [{ name, arguments }],
chunkDelayMs?, when? }`). A reply with `when` answers the first request
whose last message (its parts as JSON, a tool round's results included)
contains that text; the others answer in turn; the last repeats. A typed call takes any conversation,
tools included: the reply's text, then its tool calls (ids `call-<n>`,
finish `toolCalls`). Streamed, the text comes a word at a time, each after
`chunk_delay`, honouring cancel; with no `chunk_delay` it comes as one delta, so a
consumer that coalesces by the clock (a job's output rows) sees the same rows
whatever the load. `asked()` / `asked_typed()` are what it
was asked. The SDK's throwaway preview binds every role to it, and the
job tests run on it.
