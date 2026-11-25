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
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tree_sitter::{wasmtime, Language as TsLanguage, Parser, WasmStore};

fn engine() -> &'static wasmtime::Engine {
    static ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    ENGINE.get_or_init(wasmtime::Engine::default)
}

/// How many distinct grammars this process has loaded: a thread's lending
/// store is dropped when it moves, since a store only ever fills.
static LOADED: AtomicU64 = AtomicU64::new(0);

/// A 64-bit FNV-1a of the module's bytes, and its length: the key a loaded
/// grammar is kept under.
fn content_key(bytes: &[u8]) -> (u64, usize) {
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |acc, b| {
        (acc ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    (hash, bytes.len())
}

/// Loaded grammars (or why one didn't load), by content.
type Loaded = HashMap<(u64, usize), Result<TsLanguage, String>>;

/// Load a WebAssembly grammar. The error says why: not wasm, no
/// `tree_sitter_<name>` export, or what tree-sitter refused (an ABI it
/// doesn't read, an import a scanner may not use).
///
/// Memoised by the module's content for the life of the process: tree-sitter's
/// store adds each load's memory to a 128 MB budget and never gives it back, so
/// loading the same file at every activation would, after about a hundred
/// loads of a large grammar, refuse it. A grammar is loaded in a store of its
/// own that is dropped after the load; the language then instantiates in
/// whichever store parses with it.
pub fn load_wasm(bytes: &[u8]) -> Result<TsLanguage, String> {
    static LOADED_BY_CONTENT: OnceLock<Mutex<Loaded>> = OnceLock::new();
    let key = content_key(bytes);
    let mut loaded = LOADED_BY_CONTENT
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    if let Some(done) = loaded.get(&key) {
        return done.clone();
    }
    let done = load_wasm_in_a_new_store(bytes);
    if done.is_ok() {
        LOADED.fetch_add(1, Ordering::SeqCst);
    }
    loaded.insert(key, done.clone());
    done
}

fn load_wasm_in_a_new_store(bytes: &[u8]) -> Result<TsLanguage, String> {
    let name = wasm_grammar_name(bytes)?;
    let mut store = WasmStore::new(engine())
        .map_err(|e| format!("the WebAssembly engine didn't start: {}", e.message))?;
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
    /// With the number of grammars loaded when it was made.
    static STORE: RefCell<Option<(u64, WasmStore)>> = const { RefCell::new(None) };
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
        // `try_with`: a parser dropped while the thread's locals are being
        // torn down has nowhere to give the store back.
        if let Some(store) = self.parser.take_wasm_store() {
            let _ = STORE.try_with(|s| {
                if let Ok(mut s) = s.try_borrow_mut() {
                    *s = Some((LOADED.load(Ordering::SeqCst), store));
                }
            });
        }
    }
}

/// A parser for `grammar`, or `None` when tree-sitter refuses it (an ABI it
/// doesn't read) or a wasm store can't be made.
pub fn parser(grammar: &TsLanguage) -> Option<LangParser> {
    let mut parser = Parser::new();
    if grammar.is_wasm() {
        // A store only fills (each grammar it instantiates adds its memory),
        // so one made before the last grammar loaded is let go and remade.
        let lent = STORE
            .try_with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.take()))
            .ok()
            .flatten()
            .filter(|(loaded, _)| *loaded == LOADED.load(Ordering::SeqCst))
            .map(|(_, store)| store);
        let store = match lent {
            Some(store) => store,
            None => WasmStore::new(engine()).ok()?,
        };
        parser.set_wasm_store(store).ok()?;
    }
    let mut parser = LangParser { parser };
    if parser.set_language(grammar).is_err() {
        // The store may be full: it isn't lent again.
        drop(parser.parser.take_wasm_store());
        return None;
    }
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

    /// The same grammar loaded over and over (every activation builds the
    /// language set again) must not run the process out of wasm memory:
    /// tree-sitter's store never gives a load's memory back.
    #[test]
    fn loading_one_grammar_many_times_keeps_working() {
        // Elixir's is 1.3 MB: about 100 loads fill the store's 128 MB.
        let elixir = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../examples/extensions/elixir-pack/grammars/elixir.wasm"
        ))
        .unwrap();
        let first = load_wasm(&elixir).unwrap();
        for _ in 0..200 {
            let again = load_wasm(&elixir).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(
                again.id_for_node_kind("call", true),
                first.id_for_node_kind("call", true)
            );
        }
        let mut p = parser(&first).unwrap();
        assert!(p.parse("defmodule A do\nend\n", None).is_some());
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
