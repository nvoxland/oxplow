# AI providers and roles

This doc covers oxplow's own access to models over their APIs: the
providers you configure, the roles that decide which model does what, the
`ai_*` functions sources and views call, and how spend is tracked.

> **Status: target design (epic tsk275).** Nothing here is implemented
> yet; oxplow makes **no** direct model API calls today. The build is
> tsk279. When a piece ships, move it from "target" to "current" here,
> in the same commit.

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
- Keys are stored in the **OS keychain**, never in the repo and never under
  the project dir.
- Provider kinds: `anthropic`, `openai`, `openrouter`, `openai-compatible`
  (base URL), `typesafe`.
- Model catalog from models.dev (cached); manual entries are allowed.

## Roles

Extensions and core refer to **roles, never to models**. Each role maps to
`{provider, model, params}`. Defaults are global; a project can override
them in `.oxplow/project.yaml`.

| Role | Used for |
|---|---|
| `main` | general reasoning |
| `fast` | cheap, quick generation |
| `summarize` | summaries of sessions, efforts, changes |
| `embed` | embeddings |
| `decide` | typed questions with probabilities (Jev-shaped) |
| `review` | second-opinion review by a different model |

## Client

- A Rust crate, `oxplow-ai`, built on `genai`, which speaks each provider's
  native protocol and so keeps features like thinking budgets. A thin
  adapter handles Jev's typed-question API for the `decide` role.
- Per-role daily budget caps. A call over budget fails and shows in
  Settings → AI.
- Every call is recorded as an `oxplow.ai_call` fact (role, provider,
  model, tokens, latency, estimated cost, calling extension). AI spend is
  therefore itself queryable in the [semantic layer](./semantic-layer.md).

## `ai_*` functions

Sources and views use `ai_decide(role, input, questions)`, `ai_score`,
`ai_summarize` and `ai_embed`.

Results are **cached as facts, keyed by a hash of the input**. Views never
call a model when they render; they read cached results. This keeps views
fast, deterministic and cheap.

MCP exposes `list_ai_roles` and `ai_decide`, so the doing-agent can ask a
cheap model a typed question.

## Settings → AI

One page answers "which model is used for what":

- providers (connect, test);
- roles (assign a model);
- usage and spend this week, per extension;
- budget caps.
