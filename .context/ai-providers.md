# AI providers and roles

This doc covers oxplow's own access to models over their APIs: the
providers you configure, the roles that decide which model does what, the
`ai_*` functions sources and lenses call, and how spend is tracked.

> **Status (epic tsk275):** the core is built: providers, keychain keys,
> roles, the client, budgets and call records (`oxplow-ai`,
> `oxplow-app/src/ai_service.rs`, `v_ai_call`). **Not yet:** the Settings → AI
> page, IPC and MCP tools (tsk300), project role overrides from
> `project.yaml` (`AiService::set_overrides` exists, nothing calls it yet),
> the `ai_*` functions with fact caching, and a models.dev catalog. Sections
> below say which parts are target design.

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
    - { id: local, kind: openai-compatible, baseUrl: http://localhost:11434/v1 }
    - { id: ts, kind: typesafe }
  roles:
    summarize: { provider: or, model: openai/gpt-5-mini, dailyBudgetUsd: 1.0 }
    decide:    { provider: ts, model: jev-latest }
  ```

- Keys are stored in the **OS keychain** (`keyring` 4.2, service
  `net.voxland.oxplow`, account = provider id), never in the repo and never
  under the project dir. A provider with no key is called without auth
  (local servers). `SecretStore` is a trait; tests use `MemorySecrets`.
  Gotcha: on macOS an unsigned dev build is a "different app" after each
  rebuild, so the keychain may prompt again for access.
- Provider kinds: `anthropic`, `openai`, `openrouter`, `openai-compatible`
  (needs `baseUrl`), `typesafe`. Every kind accepts a `baseUrl` override.
- Target: model catalog from models.dev (cached); manual entries allowed.

## Roles

Extensions and core refer to **roles, never to models**. Each role maps to
`{provider, model, dailyBudgetUsd}`. Defaults are global; a project will be
able to override them (target: in `.oxplow/project.yaml`).

| Role | Used for |
|---|---|
| `main` | general reasoning |
| `fast` | cheap, quick generation |
| `summarize` | summaries of sessions, efforts, changes |
| `embed` | embeddings |
| `decide` | typed questions with probabilities (Jev-shaped) |
| `review` | second-opinion review by a different model |

## Client

- `oxplow_ai::client::Client`, hand-rolled on the workspace's `reqwest`
  rather than `genai`: we need three small request shapes, and owning them
  keeps Jev's API and OpenRouter's reported cost first-class.
  - Anthropic Messages: `POST {base}/v1/messages`, `x-api-key`,
    `anthropic-version: 2023-06-01`.
  - OpenAI, OpenRouter, compatible servers: `POST {base}/chat/completions`,
    Bearer key, `response_format: json_object` when JSON is asked for.
    OpenRouter is asked to include `usage.cost`, which becomes the call's cost.
  - TypeSafe Jev: `POST {base}/v1/systemone` with `{model, state,
    questions}`. A `noul` (yes/no) question has no criteria, a `choice`
    question's criteria are `{option: null}`, and a `score` question's are
    its ordered levels. OpenRouter serves Jev models at `/systemone` too.
- Two operations: `complete` (text, optionally JSON) and `decide` (typed
  questions: `noul` → probability, `choice` → choice + probabilities,
  `score` → level index + probabilities). Non-Jev models answer `decide`
  through a JSON prompt that describes the same answer shapes.
- Errors: 401/403 → `Auth`, 429 → `RateLimited`, other non-2xx → `Http`
  with the start of the body.
- Answers are parsed from `serde_json::Value` by hand, not through the
  `Answer` derive: a dependency turns on serde_json's `arbitrary_precision`,
  which breaks numbers inside internally tagged enums.
- `AiService` (in `oxplow-app`, on `Services.ai`) is what callers use:
  role → provider + model + keychain key → budget check → call → record.
- **Cost** is the provider-reported value (OpenRouter) or, for Jev, input
  tokens × $0.042/M. Other providers record no cost. **Budgets count only
  known costs**: a role on a provider that reports none is never stopped by
  its budget.
- Budgets are per role per UTC day. A call over budget fails with
  `OverBudget` before reaching the provider.
- Every call, including failures, is a row in `ai_call` / `v_ai_call`
  (role, provider, model, caller, tokens, latency, cost, ok, error), so AI
  spend is itself queryable in the [semantic layer](./semantic-layer.md).
- Tests use `oxplow_ai::testing::mock` (feature `test-support`), a local
  axum server standing in for a provider.

## `ai_*` functions (target)

Sources and lenses use `ai_decide(role, input, questions)`, `ai_score`,
`ai_summarize` and `ai_embed`.

Results are **cached as facts, keyed by a hash of the input**. Lenses never
call a model when they render; they read cached results. This keeps lenses
fast, deterministic and cheap.

MCP exposes `list_ai_roles` and `ai_decide`, so the doing-agent can ask a
cheap model a typed question.

## Settings → AI (target)

One page answers "which model is used for what":

- providers (connect, test);
- roles (assign a model);
- usage and spend this week, per extension;
- budget caps.
