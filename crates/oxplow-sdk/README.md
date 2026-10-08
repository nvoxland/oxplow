# oxplow-sdk

Scaffold, check and test oxplow extensions, and the provider
conformance kit. It's the library behind:

- `oxplow extension new | check | test` (the CLI in the desktop binary),
- the `validate_extension` RPC and MCP tool,
- `save_lens` (Keep This).

All three print problems the same way, `file:line: what — fix`, so a
person or a coding agent gets one answer wherever they meet it.

| Module | What it does |
|---|---|
| `lib.rs` | `scaffold` (what `new` writes for each kind) and `check` (load the extension, dry-run its models, commands, lenses and advisories) |
| `extension_test` | `test`: on a throwaway oxplow, the check, each intent example's fixture, `questions.yaml`, and each declared provider |
| `conformance` | the provider kit: a `ReferenceClient` that drives a provider over the wire, validates every message against the protocol's schemas, records a golden transcript, and runs its capability's suite |
| `answerability` | checks that an agent reading the right skill can answer the questions an area of oxplow is for |

User docs: [Extensions](../../docs/reference/extensions.md) and
[Provider protocol](../../docs/reference/provider-protocol.md). Design
notes for contributors: `.context/extensions.md` "The SDK" and
`.context/providers.md` "The conformance kit".

Tests: `bun run test:fast -p oxplow-sdk`. `tests/just_works.rs` checks
that every scaffold checks clean and passes `test` as written.
