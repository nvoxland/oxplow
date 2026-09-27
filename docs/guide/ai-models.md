# AI models

Oxplow can call models itself, separately from your coding agents: to
summarize something, or to answer a quick typed question like "is this diff
risky?". You pick which model does which job in **Settings → AI**.

## Providers

Add one per service you want to use:

| Kind | What you need |
|---|---|
| Anthropic | an API key |
| OpenAI | an API key |
| OpenRouter | an API key (one key, many models) |
| Local / OpenAI-compatible | a base URL, e.g. `http://localhost:11434/v1` for Ollama |
| TypeSafe (Jev) | an API key |

Keys go to your OS keychain, never to a file. Oxplow only shows whether a
key is saved. Providers and roles live in `ai.yaml` in oxplow's config
directory and apply to all your projects.

**Test** makes one small call with the model you name, so you can check
the key, URL and model name before using them.

## Roles

Oxplow and extensions ask for a role, not a model:

| Role | Used for |
|---|---|
| `main` | general reasoning |
| `fast` | cheap, quick generation |
| `summarize` | summaries of sessions, efforts and changes |
| `embed` | embeddings |
| `decide` | typed yes/no, choice and score questions (Jev fits well) |
| `review` | a second opinion from a different model |

A role with no model assigned just isn't available. Anything that needs it
says so.

## What your agent can do

Your coding agent sees three MCP tools:

- `list_ai_roles`: what's configured.
- `ai_decide`: ask the `decide` model typed questions about some text.
- `ai_summarize`: summarize text with the `summarize` model.

It can't change providers, roles or keys.

## Seeing the calls

**Recent Calls** in Settings → AI shows the last week by role and caller.
Every call is also in the `v_ai_call` view, so you can query it in Explore
Data or build a [lens](lenses.md) over it. Oxplow doesn't track spend; use
your provider's dashboard for that.
