# AI providers and roles

This doc covers oxplow's own access to models over their APIs: the
AI providers you configure, the roles that decide which model does what, the
`ai_*` functions sources and pages call, and how calls are recorded.

> **Status:** built: providers, keychain keys, roles, the
> client, call records (`oxplow-ai`, `oxplow-app/src/ai_service.rs`,
> `v_ai_call`), Settings → AI, and the `v_ai_role` / `v_ai_provider` live models, project role overrides,
> recorded computations (`AiCompute`, `v_ai_result`, P5.E1) and the
> `ai_*` Starlark builtins for collectors (P5.E2), and each provider's
> models (`v_ai_model`, from the models.dev catalog Rig compiles in, or a
> local server's own list).
> Sections below say which parts are target design.

## Why

Terminal harnesses (Claude Code, Codex, opencode) remain the agents that do
the work. But oxplow itself benefits from calling models directly: to
classify a request's risk, score a change, summarize a session, or answer a
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

- Each declared `ai_provider` id has one connection. Its independent settings
  are `oxplow.ai.<id>.enabled` (false by default), `.baseUrl` (empty for the
  declared default endpoint), and `.apiKey` (a secret string without a default).
  Definitions come from the enabled extension capability catalog before any key
  or connection is saved. User-named duplicate connections are not supported.
  Every setting resolves local → project → global → default independently.

  ```yaml
  oxplow.ai.openrouter.enabled: true
  oxplow.ai.openai_compatible.enabled: true
  oxplow.ai.openai_compatible.baseUrl: http://localhost:11434/v1
  oxplow.ai.roles.summarize: { provider: openrouter, model: openai/gpt-5-mini }
  ```

- API keys use the shared secure settings service, with local/project identity
  including the project and global identity independent of it. Registry ownership
  is core, so extension composers cannot read or write these credentials.
  No key value is stored in YAML, inventory SQL, events, audit or undo payloads.
  macOS may prompt again after an unsigned development build changes identity.
- **A key is bound to the URL it was saved for.** Its secure value carries the
  key and normalized endpoint (empty for the kind's default). A connection
  pointed at another host gets no key; the call asks to re-save the provider in
  AI Settings. Saving a provider rebinds a kept key to its current URL.
  Provider/role display rows remain readable when a binding is obsolete or the
  secure backend is unavailable; `key_set` means a usable key is available and
  is false on these failures. This display fallback does not apply to actual
  authentication or direct config reads, which still refuse the failure. The
  shared settings inventory separately marks unavailable credential rows.
- **A provider's `kind:` is a declared model provider**: an
  `ai_provider` implementation an extension declares, registered in
  `ModelProviders` under its declared id. `oxplow-foundation` declares one
  per Rust kind ("Rust kinds over Rig" below): `anthropic`, `openai`,
  `openai_compatible`, `openrouter`, `gemini`, `mistral`, `groq`,
  `deepseek`, `xai`, `ollama`, `llamacpp`, `typesafe`. An unknown kind is
  refused by `AiService` naming the registered ones; a kind with no default
  URL (`openai_compatible`) needs a `baseUrl`, and every kind accepts one
  as an override. `AiConfig::validate` checks only the file's own shape
  (ids, role references); the kinds are the service's. Settings → AI offers
  the registered kinds (`AiSettings.kinds`: kind, title, default URL).
- Target: model catalog from models.dev (cached); manual entries allowed.

## Roles

Extensions and core refer to **roles, never to models**. Each built-in role and
active extension profile has its own atomic object setting,
`oxplow.ai.roles.<role>`, with `provider`, `model`, and optional `restrict`.
It has no default: absence enables role fallback. Local, project and global
precedence applies separately to each role, so overriding summarize preserves
an inherited default binding. Provider ids name declared implementations.

```yaml
oxplow.ai.roles.summarize: { provider: openrouter, model: openai/gpt-5-mini }
```

- Loading the config checks each binding (`validate_ai_role`); core roles are
  from `AI_ROLE_NAMES`, extension profiles use `<namespace>/<name>`. There is
  no typed copy: `AiService` and `ai_service::role_bindings` read the settings.
- `AiService` (built with the settings store) reads through it. Settings → AI keeps
  its specialized controls; registry metadata and origins remain available in
  `v_config_value`. Global edits may be shadowed by project or local overrides.

| Role | Used for |
|---|---|
| `default` | every role without a binding of its own; general reasoning: `extract` (a collector's `ai_extract`) |
| `summarize` | summaries of sessions, requests, changes (`summarize`) |
| `decide` | typed questions with probabilities, Jev-shaped (`classify`, `score`, `decide`) |

**Default is the fallback.** Every built-in role but `default` falls back
to it while it has no binding of its own (`BuiltinRole::fallback`), so
binding `default` once makes every role work; a role is bound on its own
only to differ (a cheaper Summarize). An extension's AI profile falls back
to `default` too unless it names another built-in, whose own chain then
ends at `default` (`AiConfig::resolve` follows the chain). `default` was
`main`: `oxplow.ai.roles.main` is a retired setting that says so
(`core_settings::retired_setting`); `oxplow.ai.roles.assistant` is retired
the same way (the Assistant is gone). Only an AI profile declares
`features`; a built-in role needs none. Settings shows a role's `fallback`,
and its `problem` when its binding forbids a feature a profile needs.
Unbound with an unbound fallback is `RoleNotConfigured`: "no model is
assigned to the `summarize` role or the `default` role it falls back to".

A role exists only once something asks it. A config naming a role oxplow
dropped (`fast`, `embed`, `review`: nothing asked them) fails to load,
saying so (`oxplow_ai::config::DROPPED_ROLES`; project.yaml's
`DROPPED_AI_ROLES`).

### Extension AI profiles

An extension declares its own model needs as **AI profiles**
(`ai_profiles:` in `extension.yaml`, [extensions-jobs.md](./extensions-jobs.md)):
an id, a description, an optional `fallback` built-in role (`default` when
left out; `summarize` or `decide` otherwise) and optional
`features` (`text | tool_calling | streaming | image`). So
`oxplow_ai::config::Role` is open: `Builtin(BuiltinRole)` or
`Profile { extension, name }`, written as one string (`summarize`,
`review/review`) wherever a person sees it — `global.yaml`,
`project.yaml`'s independent role keys, Settings, `ai_call.role`, `ai_result.role`.

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
  `ai_service::prepare_built_ins` at boot and on every capabilities
  refresh: each declaration's Rust kind (`oxplow_ai_rig::provider`) under
  its declared id, title and `baseUrl`. Shared pieces stay in
  `oxplow-ai`: the `Http` POST helper and its error mapping,
  `parse_answers`, `decide_via_chat`.
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
  call `ai_capabilities(role = "default")` for the selected model or
  `ai_models(role = "default")` for its provider's declared catalog. This service
  needs no paid call and is usable in pure previews. Both resolve
  the role through `AiService` without reading the keychain; providers receive
  an instance with no key. They record no `ai_call` or AI usage. The host
  checks versions and model identities and rechecks run authority while
  waiting and before returning. Other hosts must supply an AI source through
  the shared router or report that discovery is unavailable.
  `ModelCatalogV1` distinguishes unknown from a known empty catalog and
  rejects unresolved or duplicate model identities. A Rig kind reads Rig's
  built-in model catalog (models.dev data compiled into `rig-core`): a
  hosted kind lists its models, a local one (`ollama`, `llamacpp`,
  `openai_compatible`) lists what its server answers to a keyless `GET
  <base>/models` (the OpenAI list; 3 s timeout, kept 30 s per base URL, a
  failure too, so a server that is down doesn't hold every read), each with
  unknown capabilities, else an unknown catalog; a model the catalog
  doesn't list is unknown across the board. Streaming isn't a model fact
  the catalog keeps, so it stays unknown. A hosted service's live listing
  needs the key, which discovery never reads, so it isn't used.
  `AiService::provider_models` is every configured provider's catalog, the
  live `v_ai_model` dataset (provider, model, tool_calling, streaming,
  context_tokens): AI Settings suggests them for a Model. Capability data
  moves with the `rig-core` version.
- **Typed collector completion.** `ai_complete(request, role = "default")`
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
  A Rig kind streams natively ("Rust kinds over Rig"). `AiService::stream_typed_on` is
  `complete_typed_on` streamed, and records **every** exit — a cancelled
  call's `ai_call` row is `ok: false`, `error: cancelled`, its response
  the partial reply. `AiCompute::stream_typed` is the recorded
  computation: a recorded answer is handed over at once, whole, with no
  call; a cancelled or failed stream is never the recorded answer. A
  job's rounds use it ([extensions-jobs.md](./extensions-jobs.md) "Jobs");
  collectors don't stream.
- **Rust kinds over Rig.** Every model provider is a Rust kind in
  `crates/oxplow-ai-rig`, and an `ai_provider` declaration names one with
  `provider: <kind>` (`oxplow_ai_rig::KINDS`; an unknown one is a load
  error naming the known ones). The declaration gives the id `global.yaml`'s
  `kind:` names, the title and `config: { baseUrl? }`, the URL an
  instance that names none uses. Foundation's ids are the kinds' names and
  its URLs are Rig's defaults (a test keeps them equal). Every kind but
  `typesafe` is a `RigProvider`: oxplow's `ModelProvider` over
  [Rig](https://github.com/0xPlaygrounds/rig) (`rig-core` `=0.45.0`, MIT),
  one Rig provider selection each (`ProviderId::new(vendor, format)` →
  `ProviderConfig` → `completion_model`).
  - **Why Rig.** One maintained multi-provider layer instead of our own
    request mapping per vendor. The Starlark form (`providers/*.star`, the
    `oxplow-ai-providers` runner, `ops:`, `config.models`) is gone: each
    vendor's wire (tools, system text, streaming, usage, finish reasons)
    is Rig's to keep current, and adding a vendor Rig speaks is one row.
  - **The kinds.** `anthropic` (Messages), `openai` (Rig's flagship route
    for it, the Responses API), `openai_compatible` (Chat Completions at
    the instance's URL, which it must name; a dialect of our own),
    `openrouter`, `gemini`, `mistral`, `groq`, `deepseek`, `xai`, `ollama`
    and `llamacpp` (local servers), each Rig's dialect of that name;
    `typesafe` is Rust of our own (`oxplow_ai_rig::jev`), since Rig doesn't
    speak Jev.
  - **Keys and URLs.** The key is always the instance's (the keychain),
    never Rig's environment lookup. Reading a key from the macOS keychain
    asks the person (a password prompt per read unless they Always Allow;
    a dev build is a new app to macOS after each rebuild), so every
    `KeychainSecrets` goes through one process-wide `CachedSecrets`
    (`oxplow_ai::secrets`): each key is read at most once per run, a save
    or removal updates it, and `exists` answers presence by an
    attribute-only search that decrypts nothing (the Settings statuses).
    A provider's "key saved" (`ProviderStatus::key_set`) still reads it,
    cached, because the key's endpoint binding is inside the value; the base URL is the instance's, else
    the declaration's. With no key, an OpenAI-shaped request carries no
    auth header (a local server). Rig sends through our own reqwest
    clients, which never follow a redirect: one with a 120 s total timeout
    for a whole reply, one bounded only while connecting for a stream.
  - **Conversion** (`oxplow_ai_rig` `convert`). A system message is Rig's
    system message; user text and tool results are a user message; a
    `tool` message is a user message of tool results; an assistant's text
    and tool calls are an assistant message, in order. A tool result
    carries its call's tool name, which some wires need: it is taken from
    the call it answers earlier in the conversation, and a result with no
    such call is refused (`AiError::BadRequest`). A string result is
    text, anything else JSON. Tool definitions map one to one. An image
    part is refused (no asset resolver). The reply's text and tool calls
    are parts in order; reasoning and provider items with no meaning here
    are left out; an image is refused; a reply the provider marked failed
    (a refusal) is `BadResponse`. Finish: stop, length, tool calls; a
    content filter or any other reason is `other`; none is `unknown`.
    Usage is observed when either count is reported; Rig's input count
    already holds cache reads and writes.
  - **JSON mode** (`complete { json }`) adds "Reply with a single JSON
    object and nothing else." to the system text, and on a Chat
    Completions kind asks `response_format: json_object` too.
  - **Max tokens.** Rig sets a Messages request's `max_tokens` from its
    catalog; a model it doesn't list gets 4096.
  - **Streaming.** `stream_typed` reads Rig's stream: each text fragment
    is a `text` delta at once, each tool call one `toolCall` delta when
    its part ends (its arguments whole), in order; the reply is Rig's
    folded response, the same as a whole call's. Cancel drops the stream,
    its connection with it (`unless_cancelled`), and the error carries the
    deltas so far. A stream silent for 60 s is `Timeout`. A reply that
    ends before the provider ended it is `BadResponse`.
  - **Errors** (`errors::ai_error`). 401/403 or Rig's
    `InvalidAuthentication` → `Auth`, 429 → `RateLimited`, another
    status → `Http` with the start of the body, a 2xx error envelope or a
    stream's error event → `BadResponse`, a transport failure →
    `Transport` (the URL taken out: Gemini puts the key in its query), a
    request Rig can't build → `BadRequest`. The instance's key is taken
    out of every body and message, as a whole token.
  - **`decide`** keeps the trait's JSON prompt (`decide_via_chat`), except
    OpenRouter's Jev models (a model name with `jev`), asked natively at
    `{base}/systemone`. The `decide_native` feature of the declaration is
    the kind's (`openrouter`, `typesafe`).
  - **Permission** (decided 2026-10-08). Declaring an AI provider points a
    person's key at the URL it names, so its extension must declare
    `ai.compute` (the loader refuses it otherwise), approved when the
    extension is deployed — foundation's too.
  - TypeSafe Jev: `POST {base}/v1/systemone`, Bearer key, `{model, state,
    questions}`. A `noul` (yes/no) question has no criteria, a `choice`
    question's criteria are `{option: null}`, and a `score` question's are
    its ordered levels; a `noul` answer's probability is its `noul`.
  - Tests: each kind against a local stand-in for its API
    (`oxplow_ai_fake::mock`, `mock_sse`), pinning the request shape and the
    stream, in `crates/oxplow-ai-rig/src/tests.rs`; a streaming tool-call
    job end to end in `job_runs_tests`.
- Two operations: `complete` (text, optionally JSON) and `decide` (typed
  questions: `noul` → probability, `choice` → choice + probabilities,
  `score` → level index + probabilities). Non-Jev models answer `decide`
  through a JSON prompt that describes the same answer shapes. TypeSafe
  only decides (its `complete` is an error) and its connection `test` asks
  a yes/no question; OpenRouter decides natively for a `jev` model.
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
  counted (`v_contribution_run.ai_calls`, `v_ai_call.run_id`).
- Every call, including failures, is a row in `ai_call` / `v_ai_call`
  (role, provider, model, caller, tokens, latency, ok, error, and
  `run_id` when the caller supplies it). AI usage is queryable in the
  [semantic layer](./semantic-layer.md). An `AiCompute`
  computation takes a `Caller { name, run }` (a bare name converts to
  one with no run). Collectors, job rounds and the agent's AI commands
  pass the contribution run's ID and count the request on that run
  (`contribution_run.ai_calls`). The agent's call still names its thread
  as the caller. A cached result reuses the original model call's record.
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
- Tests use `oxplow_ai_fake::mock`, `mock_sse` (a reply as timed
  server-sent event chunks, held open so a reader can be cancelled
  mid-stream) and `mock_sse_rounds` (a different stream for each request,
  for tool rounds) (the dev-only `oxplow-ai-fake` crate,
  not a feature of `oxplow-ai`, so no build compiles it two ways —
  working-in-this-repo.md "Builds and `target/`"), a local axum server
  standing in for a provider. `Services::in_memory` gets
  `MemorySecrets` and a config dir under the test project's `.oxplow/`, so
  rpc/mcp tests never touch the real keychain or `global.yaml`.
- `oxplow_app::ai_service` re-exports the `oxplow-ai` types the adapters
  need (`Role`, `ProviderConfig`, `Question`, …), so `oxplow-rpc`,
  `oxplow-tauri-ipc` and `oxplow-mcp` depend only on `oxplow-app`.

## The Assistant (removed)

The Assistant was removed (2026-11-19). Its panel was too small to read an
answer. Add to Context already lets a thread's agent answer about a thing.

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
| `extract(caller, role, instructions, text, schema)` | `default`, JSON mode; the schema is in the system prompt | JSON matching `schema` (checked with `InputValidator`; a mismatch is refused, naming the pointer) |

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
are two computations). `AiService::complete_on` / `decide_on` / `complete_typed_on` /
`stream_typed_on` take the `Bound` and a `CallSite { caller, input_hash }`, so the computing call's `ai_call` row
carries the hash, and return its row id; plain `complete` / `decide`
record `input_hash` NULL.

## `ai_*` functions for sources (current, P5.E2)

An **entity collector** (a starlark derived source) calls `ai_classify(text,
labels)`, `ai_score(text, levels)`, `ai_summarize(text, focus = None)`
and `ai_extract(text, schema, instructions = "")`, each with a keyword
`role=` (defaults `decide`, `decide`, `summarize`, `default`; or one of its
extension's AI profiles by id, qualified with the run's extension) — Starlark builtins
(`crates/oxplow-script/src/ai.rs`) answered by the run's
`AiHost`, one of the services the run was given (`runtime::Hosts`,
[extensions-permissions.md](./extensions-permissions.md) "Services"). The host holds
a synchronous `dyn AiOracle`; the app's is `ai_compute::CollectorOracle`,
which blocks the script's worker thread on the runtime and asks
`AiCompute` as caller `collector:<namespace>/<id>` — so every answer is a
recorded computation (the same question on the same text is one call,
ever). Only a script whose extension declares `ai.compute` gets it: one that
names an `ai_*` builtin is refused otherwise, like an exec program needing
`process` ([semantic-layer.md](./semantic-layer.md) "Sandbox"). The script runs in a script worker
([collection.md](./collection.md) "Containing scripts"); each `ai_*` call
crosses to the host, which asks the oracle. The time it waits is left out
of the run's `timeout` (`RunClock`, the in-flight call included) but not
out of its `ceiling` (10 min of wall clock, model time included), so a
per-row loop of calls can't run for hours and hold up every scheduled
source behind it. Past either the worker is killed: no paid calls after
a timeout. A run cut off keeps
what it recorded, so its next run gets further. Who gets a model is the
services router's ([extensions-permissions.md](./extensions-permissions.md) "Services"): an
entity collector whose script names an `ai_*` builtin implies
`ai.compute`; a **fact collector** gets one when it declares
`needs: [ai.compute]` and its extension declares `ai.compute` (otherwise
it is refused before it runs); a script that composes — a command's, an effect's, a
policy's, a review's dry run — runs pure and is refused before it runs:
"`ai_summarize()`: a pure run can't use `ai.compute` (ask a model): only
a run that may call out can". A run not given a model (a report parser)
is told the need: "`ai_classify()` needs `ai.compute` in this script's
`needs`". Pages never call a model when they render; they read
what collectors recorded. The extension skill's example classifies each
turn's prompt (`turn_kind`).

## MCP (current)

- `v_ai_provider` (with `key_set`, never keys) and `v_ai_role` (every role's
  binding, what a call would use now, and whether its provider has a key),
  read live from registered effective settings and secure presence metadata;
  local/project/global file changes refresh the runtime configuration.
- `oxplow_ai_decide`: typed questions (`noul` / `choice` / `score`) about some
  text, on the `decide` role (an agent can't pick a costlier one) — a
  recorded computation (`AiCompute::decide`), returning `{ answers,
  cached }`.
- `oxplow_ai_summarize`: text through the `summarize` role, with an optional focus
  — a recorded computation (`AiCompute::summarize`), so the same text and
  focus is one call.
- What a call was asked or answered is `read` of its `ai_call` URI (the
  caller's own calls).

Each is paid with the person's key, so neither is a read-only tool (no
`read_only_hint`: a harness doesn't auto-approve it), each takes at most
`AI_TEXT_MAX` (100,000) characters of text, and each records as the
asking thread (caller its thread id, `thr3`, from the session's bearer); a
connection with no thread identity is refused. Agents can't change
providers, roles or keys: those IPC commands are UI-only in the
surface-parity manifest.

## AI Settings (Settings → AI)

One page, foundation's `ai-settings` (`AI Settings`, System), sets up all of
oxplow's AI; Settings → AI embeds it and Settings' Manage services links it.
It reads the live models `v_ai_provider`, `v_ai_role`, `v_ai_model` and
`v_ai_call` and writes with commands (`commands/ai_settings.rs`), anyone's —
a person's, their agent's, or a page's: `oxplow.ai.set_provider { id?, kind,
base_url?, key? }` (`id` is the kind, which it defaults to) (scope
`ai.settings`; `key` is a secret (`writeOnly`, commands.md "Secrets"), so a
refusal that could quote it is recorded without its words, and it goes only
to the keychain; it, `remove_provider` and `set_role` announce
`ConfigChanged`, which re-reads the models), `oxplow.ai.remove_provider { id }`
(asks first, `confirm: always`), `oxplow.ai.set_role { role, provider?,
model?, restrict? }` (neither provider nor model unbinds) and
`oxplow.ai.test_provider { id, model }` (scope `ai.test`, `spends`: one
small call on the person's key). Every form is the tab modal; the key needs
no `secret:` flag, since `set_provider` marks it `writeOnly`, which the
modal masks and never drafts.

- **Providers:** name, kind, base URL, key status and **Used by** (the roles
  whose model comes from it, own or fallen back to). Add a Provider…
  (prominent; the kinds not set up yet). A row's pencil, or a click on it, is **Edit…** (URL
  and key; an empty key keeps the saved one; its button is Save, `submit`;
  its form holds **Remove**, `also`, offered while no role is bound to it;
  Remove deletes the key). **Test…** makes one small call with a model you
  name, suggested from that provider's `v_ai_model` (a `decide` question
  for Jev); it isn't recorded in `ai_call`.
- **Models:** one row per role — the built-ins (their text is
  `BuiltinRole::description`), then each active extension's AI profiles,
  then bindings for profiles nothing declares (a problem, so they can be
  cleared) — so it is one table, not a section per role: extensions add
  rows. Two columns: **Role** (its name in bold, then what it is used for,
  `format: markdown`) and **Uses**: its own model (`anthropic · m`), its
  fallback's named (`Default's: anthropic · m`), what it would fall back to
  ("Not set up (Default has none)"), "Not set up", or a model "(set in
  project.yaml)" (`overridden`: changed there, so it offers nothing). A
  role's `problem` (its model or its `restrict` lacks what the role needs;
  a binding for a profile nothing declares) follows the model, the cell in
  the warning tone. Default is the first row: every other role falls back
  to it (a profile through the built-in it names), so one model set on
  Default serves them all. A row's pencil, or a click, is **Choose Model…** (provider and model;
  Save; the binding's `restrict` kept as it is; the Model suggests every
  provider's models, labelled with theirs, for every role — open choices,
  any id may be typed). **Use the Fallback Model** unbinds the role, offered only
  while its fallback has a model, so it never leaves a role with none.
- **This week's calls:** `v_ai_call` of the last 7 days by role and caller.

Fixture `ai-settings-sets-up-providers-and-each-roles-model`. The raw
settings behind it (`oxplow.ai.*`) stay in Settings' AI category.

## Host transport and live authority

Every provider HTTP client (oxplow-ai's `Http`, the ones Rig sends
through) refuses redirects; a provider request cannot carry credentials or
content to a redirected destination. Transport errors strip
URL details, including query values that hold a key, and an echoed key is
taken out of error bodies.
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

The standalone `AiConfig::load` compatibility reader rejects retired
`ai.providers` and `ai.roles` keys in `global.yaml`, with current Settings → AI
key forms in its error. It does not migrate or remove those values or credentials.
