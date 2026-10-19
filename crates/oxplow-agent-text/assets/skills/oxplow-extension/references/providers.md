# Testing an external provider

An extension with an **external provider** (`providers:` — a program that
   connects an outside system, such as an issue tracker; private
   extensions only) is tested with `oxplow extension test <name>`:
   `oxplow extension new provider <name> [--capability <cap>]` scaffolds
   one (its `provider.json` declarations from the capability's contract, a
   stub `bin/provider` to replace, and the fixtures).
   The test runs `check`, then the provider: its `initialize` must equal
   `provider.json`, its `check` must accept `fixtures/provider-<id>.yaml`'s
   `config`, each intent example's fixture (`input: { command, input }`,
   `expect`, `$any` matching anything) is invoked, every message must
   match the protocol, the session must match the golden
   `fixtures/transcripts/<id>.jsonl` (`--bless` writes it when a change is
   intended — commit it), and its capability's conformance suite must pass.
   A `ref_kinds:` entry may add `searchable: <model>` (a model of yours
   with `ref`, `title`, `body`): its rows are then found by the launcher's
   search and open the kind's page.
   An effect's `on:` (and a collector's `trigger: { on: [...] }`) may
   name another extension's event type (`acme_pr.merged`): `check` warns,
   and the extension shows an error while no enabled extension registers
   that type. The payload you get is at the owner's newest version.
   A provider's `credentials:` entry is a name (the person pastes its
   value) or `{ name, oauth: { authorize_url, token_url, client_id,
   scopes } }` (the person signs in on Settings → Integrations; oxplow
   runs the flow and renews the token, and your program reads the access
   token from the env var of that name — answer `Auth` (-32002) when the
   service refuses it and oxplow renews it and calls again). You can't
   sign in or set a credential yourself.
   The extension declares `permissions.process` (and its `network`
   hosts and `secrets`), which a person approves when it is deployed, and
   enables the provider in Settings → Integrations; you can't do either.
   A work-items provider is a **backend for the project's own work**,
   with the visibility the person wants (a local tracker like beads), not
   a team's tracker: when it's the active one, every new item goes there.
   In the oxplow repo, `tests-e2e/fixtures/extension/` (its manifest and
   `provider.json`) and `crates/oxplow-provider-fake` (the program) are a
   complete one: copy their shape.
   An existing **MCP server** can be the provider instead of a program:
   `adapter: { mcp: { command: [bin/server] }, mapping: mcp/x.star,
   tools: mcp/tools.json }` in place of `entry` — oxplow's adapter runs
   the server, `check` refuses it unless its tools equal the pinned
   `tools.json`, and `transform(x)` in the mapping turns each verb into a
   tool call (`x.phase` `invoke`) and the tool's output into the answer
   (`invoked`; `read` / `records` for a collector). A server that
   already runs elsewhere is `mcp: { url: https://…, auth: NAME }`
   instead of `command`: https only (http on loopback), its host in
   `network`, and `auth` one of the provider's `credentials` (sent as the
   bearer token).
