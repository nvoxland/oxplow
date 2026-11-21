# oxplow-json

What oxplow knows about JSON, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has JSON files (`oxplow.extensions.oxplow-json.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares `json`: the file extensions and the Mason package of its language server. It has no grammar, so nothing parses it: oxplow knows it by its server only.
