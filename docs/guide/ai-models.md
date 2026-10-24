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
| Google Gemini | an API key |
| Mistral, Groq, DeepSeek, xAI | an API key |
| Ollama, llama.cpp server | nothing, if it runs on this machine at its usual port |
| Any OpenAI-compatible API | a base URL, e.g. LM Studio's or vLLM's |
| TypeSafe (Jev) | an API key |

Keys go to your OS keychain, never to a file. Oxplow only shows whether a
key is saved. Providers and roles live in `ai.yaml` in oxplow's config
directory and apply to all your projects.

**Test** makes one small call with the model you name, so you can check
the key, URL and model name before using them.

### Provider permissions

Each provider kind is code in oxplow itself. The extension that declares a
provider names its kind and the URL it calls by default. A call carries
your key and your prompts, so that extension declares
`permissions: { ai: true }`, including the extension that comes with
oxplow. You approve that when you deploy the extension (Settings →
Extensions).

An extension can add a provider of its own the same way (see the
[Extensions reference](../reference/extensions.md)).

## Roles

Oxplow and extensions ask for a role, not a model:

| Role | Used for |
|---|---|
| `main` | general reasoning |
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

### Restricting what a model may do

A role's model can be kept from using tools, SQL, streaming or image
input, whatever asks it. In Settings → AI, uncheck what the role's model
**Allows**; in a file, list what it may not do:

```yaml
ai:
  roles:
    summarize: { provider: openrouter, model: openai/gpt-5-mini, restrict: [tools, sql] }
```

A request that needs something the model is restricted from fails before
anything is sent, naming the role and the restriction. Extensions choose
which role or profile they use, but they can't lift a restriction.
`oxplow extension check` warns when an extension's AI profile needs
something its model is restricted from.

## What oxplow uses them for

- **Decisions and claims Oxplow noticed.** When an effort closes, a model
  reads that effort's conversation, final answers and tool calls. It lists
  decisions the agent made without recording them, and the claims its
  answers made about its work ("the tests pass", "no behavior change").
  They show on the effort's review, next to the decisions the agent did
  record; you confirm or dismiss a decision, and verify or dismiss a
  claim. They're guesses, and your agent never sees them. The model is the
  one you bind to `oxplow-bundled/reasoning` in Settings → AI, or your
  `main` model until you do. Nothing runs until one of them has a model,
  and its extension is deployed with the permissions it declares (Settings →
  Extensions). **Infer Again** on the list asks once more, say after you bind
  another model.

## What your agent can do

Your coding agent can read what's configured and ask your models two things:

- `v_ai_role` and `v_ai_provider` (through `oxplow_query_sql`): what's configured, and
  whether each provider has a key — never the key.
- `oxplow_ai_decide`: ask the `decide` model typed questions about some text.
- `oxplow_ai_summarize`: summarize text with the `summarize` model.

Both spend your key. Each answer is recorded against the thread that
asked, and asking the same thing again reuses it instead of calling the
model. Neither is marked read-only, so your agent's own permission
prompts treat them like any other tool that acts. Extension collectors
that ask a model (`ai_classify`, `ai_summarize` and the rest) spend your key
the same way.

It can't change providers, roles or keys.

## Seeing the calls

**Recent Calls** in Settings → AI shows the last week by role and caller.
Every call is also in the `v_ai_call` view, so you can query it in Explore
Data or build a [lens](lenses.md) over it. Oxplow doesn't track spend; use
your provider's dashboard for that.
