# AI models

Oxplow can call models itself, separately from your coding agents: to
summarize something, or to answer a quick typed question like "is this diff
risky?". You set up providers and pick which model does which job on the **AI Settings**
page (also **Settings → AI**).

## Providers

**Add a Provider…** sets one up. A provider's pencil (or a click on its
row) edits its URL or key, or removes it. Each provider has one connection:

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
key is saved. Provider preferences and role bindings use the shared settings levels: local,
project and global. Global preferences live in `global.yaml` in oxplow's
configuration directory; on macOS this is
`~/Library/Application Support/net.voxland.oxplow/`, unless `OXPLOW_HOME` overrides it.
Each provider has independent `oxplow.ai.<provider>.enabled`, `.baseUrl` and
secret `.apiKey` settings. Changing the endpoint requires saving a key for the
new destination.

**Test…** makes one small call with the model you name, so you can check
the key, URL and model name before using them. As you type a model, oxplow
suggests the provider's models: a hosted service's from the catalog oxplow
ships, a local server's (Ollama, llama.cpp, any OpenAI-compatible one) from
the server itself. You can always type a model id it doesn't list.

### Provider permissions

Each provider kind is code in oxplow itself. The extension that declares a
provider names its kind and the URL it calls by default. A call carries
your key and your prompts, so that extension declares
`permissions: [ai.compute]`, including the extension that comes with
oxplow. You approve that when you deploy the extension (Settings →
Extensions).

An extension can add a provider of its own the same way (see the
[Extensions reference](../reference/extensions.md)).

## Roles

Oxplow and extensions ask for a role, not a model:

| Role | Used for |
|---|---|
| `default` | every role without a model of its own, and general reasoning |
| `summarize` | summaries of sessions, requests and changes |
| `decide` | typed yes/no, choice and score questions (Jev fits well) |

Set **Default** first: every other role without a model of its own uses
Default's, so one model makes everything work. Give a role its own model
only to make it different, such as a cheaper one for `summarize`. Under
**Models**, a role's pencil (or a click on its row) chooses its model, and
**Use the Fallback Model** goes back to Default's. With no model on
Default either, a role isn't available, and anything that needs it says so.

A project can set roles for everyone working on it in `.oxplow/project.yaml`:

```yaml
oxplow.ai.roles.summarize: { provider: openrouter, model: openai/gpt-5-mini }
```

The provider id names a declared service, so each person enables that service
and supplies their own credential. AI Settings shows these roles as "Set in
project.yaml", and you change them there.

### Restricting what a model may do

A role's model can be kept from using tools, SQL, streaming or image
input, whatever asks it. List what it may not do in the role's setting
(choosing its model on AI Settings keeps the list):

```yaml
oxplow.ai.roles.summarize: { provider: openrouter, model: openai/gpt-5-mini, restrict: [tools, sql] }
```

A request that needs something the model is restricted from fails before
anything is sent, naming the role and the restriction. Extensions choose
which role or profile they use, but they can't lift a restriction.
`oxplow extension check` warns when an extension's AI profile needs
something its model is restricted from.

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

**This week's calls** on AI Settings shows the last week by role and caller.
Every call is also in the `v_ai_call` view, so you can query it in Explore
Data or build a [page](pages.md) over it. Oxplow doesn't track spend; use
your provider's dashboard for that.
