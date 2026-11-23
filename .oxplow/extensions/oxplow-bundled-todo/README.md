# oxplow-bundled-todo

Oxplow's own contributions that have not been reviewed yet. This
extension is off (`oxplow.extensions.oxplow-bundled-todo.enabled: false` in
`.oxplow/project.yaml`), it does not ship, and nothing may depend on it.
Each piece moves to `extensions/oxplow-bundled` when it is cleaned up and
has fixtures. When nothing is left, this extension goes away. Do not add
new work here. `.context/extensions-bundled.md` "The `oxplow-bundled`
extension" has the rules for a move.

## What is still here

- **Page**: Explore Data.
- **Custom component**: Explore Data.

`desktop-tests/` and `e2e-specs/` are the old frontend and browser tests of
these pages. They are reference for a move, not specs to restore.
