`toml.wasm` is the TOML tree-sitter grammar (tree-sitter-grammars/tree-sitter-toml,
MIT) compiled to WebAssembly, as Zed's TOML extension ships it. The wasm loader's
tests parse with it; no shipped language uses it.
