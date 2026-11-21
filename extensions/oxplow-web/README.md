# oxplow-web

What oxplow knows about HTML and CSS, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has HTML and CSS files (`oxplow.extensions.oxplow-web.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares `html`, `css`: the file extensions and the Mason package of its language server. It has no grammar, so nothing parses it: oxplow knows it by its server only.
