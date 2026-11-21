# oxplow-lua

What oxplow knows about Lua, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has Lua files (`oxplow.extensions.oxplow-lua.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares `lua`: the file extensions and the Mason package of its language server. It has no grammar, so nothing parses it: oxplow knows it by its server only.
