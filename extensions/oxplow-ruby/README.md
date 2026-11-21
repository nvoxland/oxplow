# oxplow-ruby

What oxplow knows about Ruby, as a language pack that ships with oxplow and is off by default. Turn it on in a project that has Ruby files (`oxplow.extensions.oxplow-ruby.enabled: true`); oxplow suggests it when it sees them.

- `languages:` in `extension.yaml` declares `ruby`: the file extensions and the Mason package of its language server. It has no grammar, so nothing parses it: oxplow knows it by its server only.
- `test_conventions:`, `file_kinds:` and `run_patterns:` name this stack's spec files and skips, Gemfile and lockfile, RuboCop and RSpec files, and `rspec`.
