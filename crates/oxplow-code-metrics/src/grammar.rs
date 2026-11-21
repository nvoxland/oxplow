//! Grammars and the parsers over them.
//!
//! A language's grammar is either compiled into oxplow ([`native_grammar`],
//! for the shipped packs) or a WebAssembly file its extension brings
//! (`grammar: { wasm: grammars/x.wasm }`), loaded here: one wasmtime engine
//! per process, a loading store behind a lock, and the grammar's name read
//! from its own `tree_sitter_<name>` export, so the file alone says what it
//! is. Only lexing runs in wasm; the parse tables are copied out, so a
//! wasm language is used like a native one.
//!
//! Every parse goes through [`parser`]: a wasm language needs a `WasmStore`
//! on its parser, and making one is costly, so each thread keeps one and
//! lends it to each parser it makes ([`LangParser`] gives it back on drop).
//!
//! [`native_grammar`]: crate::native_grammar

use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::sync::{Mutex, OnceLock};

use tree_sitter::{wasmtime, Language as TsLanguage, Parser, WasmStore};

fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(wasmtime::Engine::default)
}

/// Load a WebAssembly grammar. The error says why: not wasm, no
/// `tree_sitter_<name>` export, or what tree-sitter refused (an ABI it
/// doesn't read, an import a scanner may not use).
pub fn load_wasm(bytes: &[u8]) -> Result<TsLanguage, String> {
    let name = wasm_grammar_name(bytes)?;
    static LOADER: OnceLock<Result<Mutex<WasmStore>, String>> = OnceLock::new();
    let loader = LOADER
        .get_or_init(|| {
            WasmStore::new(engine())
                .map(Mutex::new)
                .map_err(|e| e.message)
        })
        .as_ref()
        .map_err(|e| format!("the WebAssembly engine didn't start: {e}"))?;
    let mut store = loader.lock().unwrap_or_else(|p| p.into_inner());
    store
        .load_language(&name, bytes)
        .map_err(|e| format!("the grammar `{name}` didn't load: {}", e.message))
}

/// The `<name>` of a grammar module's `tree_sitter_<name>` export (its
/// external scanner's exports aside).
pub fn wasm_grammar_name(bytes: &[u8]) -> Result<String, String> {
    let names = wasm_exports(bytes).ok_or("not a WebAssembly module")?;
    let mut grammars = names.iter().filter_map(|n| {
        n.strip_prefix("tree_sitter_")
            .filter(|rest| !rest.contains("_external_scanner_"))
    });
    match (grammars.next(), grammars.next()) {
        (Some(name), None) => Ok(name.to_string()),
        (None, _) => Err("the module exports no `tree_sitter_<name>` grammar".into()),
        (Some(a), Some(b)) => Err(format!(
            "the module exports more than one grammar (`tree_sitter_{a}`, `tree_sitter_{b}`)"
        )),
    }
}

/// The names a WebAssembly module exports, from its export section; `None`
/// when the bytes aren't a module.
fn wasm_exports(bytes: &[u8]) -> Option<Vec<String>> {
    fn leb(bytes: &[u8], at: &mut usize) -> Option<usize> {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = *bytes.get(*at)?;
            *at += 1;
            value |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                return Some(value);
            }
            shift += 7;
            if shift > 35 {
                return None;
            }
        }
    }
    if bytes.get(..8)? != b"\0asm\x01\0\0\0" {
        return None;
    }
    let mut at = 8;
    while at < bytes.len() {
        let id = bytes[at];
        at += 1;
        let size = leb(bytes, &mut at)?;
        let end = at.checked_add(size).filter(|e| *e <= bytes.len())?;
        if id == 7 {
            let mut names = Vec::new();
            let count = leb(bytes, &mut at)?;
            for _ in 0..count {
                let len = leb(bytes, &mut at)?;
                let name = std::str::from_utf8(bytes.get(at..at.checked_add(len)?)?).ok()?;
                at += len + 1; // the name, then its kind byte
                leb(bytes, &mut at)?; // its index
                names.push(name.to_string());
            }
            return Some(names);
        }
        at = end;
    }
    Some(Vec::new())
}

thread_local! {
    /// The store this thread lends to the parsers it makes for wasm
    /// languages.
    static STORE: RefCell<Option<WasmStore>> = const { RefCell::new(None) };
}

/// A parser set to one language; derefs to tree-sitter's [`Parser`]. A wasm
/// language's store goes back to the thread when it drops.
pub struct LangParser {
    parser: Parser,
}

impl Deref for LangParser {
    type Target = Parser;
    fn deref(&self) -> &Parser {
        &self.parser
    }
}

impl DerefMut for LangParser {
    fn deref_mut(&mut self) -> &mut Parser {
        &mut self.parser
    }
}

impl Drop for LangParser {
    fn drop(&mut self) {
        if let Some(store) = self.parser.take_wasm_store() {
            STORE.with(|s| *s.borrow_mut() = Some(store));
        }
    }
}

/// A parser for `grammar`, or `None` when tree-sitter refuses it (an ABI it
/// doesn't read) or a wasm store can't be made.
pub fn parser(grammar: &TsLanguage) -> Option<LangParser> {
    let mut parser = Parser::new();
    if grammar.is_wasm() {
        let store = match STORE.with(|s| s.borrow_mut().take()) {
            Some(store) => store,
            None => WasmStore::new(engine()).ok()?,
        };
        parser.set_wasm_store(store).ok()?;
    }
    let mut parser = LangParser { parser };
    parser.set_language(grammar).ok()?;
    Some(parser)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &[u8] = include_bytes!("../testdata/toml.wasm");

    /// A wasm grammar loads, and parses like a native one: its node kinds
    /// are there, and a parser made twice on one thread reuses its store.
    #[test]
    fn a_wasm_grammar_loads_and_parses() {
        let toml = load_wasm(TOML).unwrap();
        assert!(toml.is_wasm());
        assert_ne!(toml.id_for_node_kind("table", true), 0);
        for _ in 0..2 {
            let mut p = parser(&toml).unwrap();
            let tree = p.parse("[a]\nb = 1\n", None).unwrap();
            assert_eq!(tree.root_node().kind(), "document");
            assert!(!tree.root_node().has_error());
        }
        assert!(STORE.with(|s| s.borrow().is_some()));
    }

    /// The grammar's name comes from its export; bytes that aren't a
    /// grammar module say why.
    #[test]
    fn a_grammar_is_named_by_its_export() {
        assert_eq!(wasm_grammar_name(TOML).unwrap(), "toml");
        assert!(wasm_grammar_name(b"not wasm")
            .unwrap_err()
            .contains("not a WebAssembly module"));
        // An empty module: the header and no sections.
        assert!(wasm_grammar_name(b"\0asm\x01\0\0\0")
            .unwrap_err()
            .contains("no `tree_sitter_<name>`"));
        assert!(load_wasm(b"\0asm\x01\0\0\0").is_err());
    }
}

/// How a wasm grammar costs against a native one: its load, and parsing
/// the same amount of code. `cargo nextest run … --run-ignored only` prints
/// it; `.context/performance.md` keeps the numbers.
#[cfg(test)]
mod timing {
    use std::time::Instant;

    #[test]
    #[ignore]
    fn wasm_grammar_cost() {
        let bytes = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../extensions/oxplow-clojure/grammars/clojure.wasm"
        ))
        .unwrap();
        let t = Instant::now();
        let clojure = super::load_wasm(&bytes).unwrap();
        let load = t.elapsed();
        let clj = "(defn f [x]\n  (if (pos? x) (map inc [1 2 3]) {:a 1 :b \"s\"}))\n".repeat(5000);
        let rust = "fn f(x: i32) -> i32 {\n    if x > 0 { [1, 2, 3].iter().map(|n| n + 1).sum() } else { 0 }\n}\n".repeat(5000);
        let native = crate::native_grammar("rust").unwrap();
        let time = |lang: &tree_sitter::Language, text: &str| {
            let mut p = super::parser(lang).unwrap();
            let t = Instant::now();
            for _ in 0..5 {
                p.parse(text, None).unwrap();
            }
            t.elapsed() / 5
        };
        let first = Instant::now();
        drop(super::parser(&clojure));
        let first = first.elapsed();
        eprintln!(
            "load {load:?}; first parser {first:?}; clojure wasm {:?} for {} KB; rust native {:?} for {} KB",
            time(&clojure, &clj),
            clj.len() / 1024,
            time(&native, &rust),
            rust.len() / 1024
        );
    }
}
