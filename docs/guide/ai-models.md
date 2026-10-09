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

### Approving a provider

Each provider is a small script that shapes the calls oxplow makes to
that service, including the ones that come with oxplow. A call carries
your key and your prompts, so a provider runs only once you've approved
its script in **Settings → Data → Programs**. **Read the script** shows
you what it does. Until then, calls through it fail and say where to
approve it.

An upgrade that changes a provider's script asks again: AI calls through
it stop until you approve the new version.

An extension can add a provider of its own the same way (see the
[Extensions reference](../reference/extensions.md)).

## Roles

Oxplow and extensions ask for a role, not a model:

| Role | Used for |
|---|---|
| `main` | general reasoning, such as finding the decisions an effort made |
| `summarize` | summaries of sessions, efforts and changes |
| `decide` | typed yes/no, choice and score questions (Jev fits well) |

A role with no model assigned just isn't available. Anything that needs it
says so.

A project can set roles for everyone working on it in `.oxplow/project.yaml`:

```yaml
ai:
  roles:
    summarize: { provider: openrouter, model: openai/gpt-5-mini }
```

The provider name refers to each person's own providers, so everyone needs
one with that name. Settings → AI marks these roles "Set by this project".

## What oxplow uses them for

- **Decisions Oxplow Noticed.** When an effort closes, the `summarize` model
  reads that effort's conversation and tool calls and lists decisions the
  agent made without recording them. They show on the effort's review,
  next to the decisions the agent did record. They're guesses, and your
  agent never sees them. Nothing runs until `summarize` has a model.

## What your agent can do

Your coding agent sees three MCP tools:

- `list_ai_roles`: what's configured.
- `ai_decide`: ask the `decide` model typed questions about some text.
- `ai_summarize`: summarize text with the `summarize` model.

Both spend your key. Each answer is recorded against the thread that
asked, and asking the same thing again reuses it instead of calling the
model. Neither is marked read-only, so your agent's own permission
prompts treat them like any other tool that acts.

It can't change providers, roles or keys.

## Seeing the calls

**Recent Calls** in Settings → AI shows the last week by role and caller.
Every call is also in the `v_ai_call` view, so you can query it in Explore
Data or build a [lens](lenses.md) over it. Oxplow doesn't track spend; use
your provider's dashboard for that.
