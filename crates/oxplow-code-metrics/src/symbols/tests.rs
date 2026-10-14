use super::*;

/// `(kind, id)` of each definition in `src`, in source order.
pub(super) fn ids(path: &str, src: &str) -> Vec<(String, String)> {
    extract(path, src)
        .into_iter()
        .map(|d| (d.kind.as_str().to_string(), d.id))
        .collect()
}

pub(super) fn expect(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, i)| (k.to_string(), i.to_string()))
        .collect()
}

#[test]
fn every_built_in_symbol_query_compiles_against_its_grammar() {
    for support in crate::registry() {
        let spec = symbol_spec(support.language);
        assert!(
            spec.is_ok(),
            "{}: {}",
            support.language.name(),
            spec.err().unwrap()
        );
    }
}

#[test]
fn rust_definitions_and_their_ids() {
    let src = r#"
pub struct Services { db: Db, pub(crate) name: String }
impl Services {
    pub fn boot() {}
    fn new() -> Self { todo!() }
}
impl Display for Parser {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result { Ok(()) }
}
impl<T> Stack<T> { fn push(&mut self, t: T) {} }
const MAX_FILES: usize = 3;
static REGISTRY: u8 = 0;
enum Kind { Function, Class(u8) }
trait Shape { fn area(&self) -> f64; type Unit; }
mod tests { fn parses() { let f = |x: i32| x + 1; } }
macro_rules! bail { () => {} }
type Alias = u8;
fn outer() { fn inner() {} }
#[cfg(unix)] fn plat() {}
#[cfg(windows)] fn plat() {}
"#;
    assert_eq!(
        ids("src/lib.rs", src),
        expect(&[
            ("struct", "Services"),
            ("field", "Services::db"),
            ("field", "Services::name"),
            ("method", "Services::boot"),
            ("method", "Services::new"),
            ("method", "<Parser as Display>::fmt"),
            ("method", "Stack::push"),
            ("constant", "MAX_FILES"),
            ("constant", "REGISTRY"),
            ("enum", "Kind"),
            ("enum_member", "Kind::Function"),
            ("enum_member", "Kind::Class"),
            ("trait", "Shape"),
            ("method", "Shape::area"),
            ("type", "Shape::Unit"),
            ("module", "tests"),
            ("function", "tests::parses"),
            ("macro", "bail"),
            ("type", "Alias"),
            ("function", "outer"),
            ("function", "outer::inner"),
            ("function", "plat"),
            ("function", "plat~2"),
        ])
    );
}

#[test]
fn python_definitions_and_their_ids() {
    let src = r#"
TIMEOUT = 30
class Builder:
    retries = 3
    def __init__(self): pass
    @property
    def run(self):
        def helper(): pass
        return lambda x: x
def main(): pass
def main(): pass
"#;
    assert_eq!(
        ids("scripts/build.py", src),
        expect(&[
            ("variable", "TIMEOUT"),
            ("class", "Builder"),
            ("field", "Builder::retries"),
            ("method", "Builder::__init__"),
            ("method", "Builder::run"),
            ("function", "Builder::run::helper"),
            ("function", "main"),
            ("function", "main~2"),
        ])
    );
}

#[test]
fn java_callables_carry_their_parameter_types() {
    let src = r#"
package com.acme;
class Parser {
    private int depth = 0;
    Parser(String source) {}
    int parse(String s, int n) { return n; }
    int parse(List<String> xs) { return 0; }
    void log(String... parts) {}
    class Inner { int x; }
    enum Mode { FAST, SLOW }
}
interface Shape { double area(); }
"#;
    assert_eq!(
        ids("src/main/java/com/acme/Parser.java", src),
        expect(&[
            ("class", "Parser"),
            ("field", "Parser::depth"),
            ("constructor", "Parser::Parser(String)"),
            ("method", "Parser::parse(String,int)"),
            ("method", "Parser::parse(List<String>)"),
            ("method", "Parser::log(String...)"),
            ("class", "Parser::Inner"),
            ("field", "Parser::Inner::x"),
            ("enum", "Parser::Mode"),
            ("enum_member", "Parser::Mode::FAST"),
            ("enum_member", "Parser::Mode::SLOW"),
            ("interface", "Shape"),
            ("method", "Shape::area()"),
        ])
    );
}

#[test]
fn a_definition_records_where_its_name_is_and_its_whole_extent() {
    let src = "struct A;\n\nimpl A {\n    pub fn go(&self) {\n        1;\n    }\n}\n";
    let go = extract("a.rs", src)
        .into_iter()
        .find(|d| d.id == "A::go")
        .unwrap();
    assert_eq!(go.name, "go");
    assert_eq!(go.container, vec!["A".to_string()]);
    assert_eq!((go.name_line, go.name_col), (4, 12));
    assert_eq!((go.start_line, go.start_col), (4, 5));
    assert_eq!((go.end_line, go.end_col), (6, 6));
}

#[test]
fn an_unsupported_or_unparseable_file_has_no_definitions() {
    assert!(extract("notes.txt", "fn a() {}").is_empty());
}

#[test]
fn a_query_capturing_an_unknown_name_is_refused() {
    let err = SymbolSpec::compile(
        crate::Language::Rust,
        "(function_item name: (identifier) @name) @definition.lambda",
        None,
    )
    .err()
    .unwrap();
    assert!(err.contains("`definition.lambda`"), "{err}");
    let err = SymbolSpec::compile(
        crate::Language::Rust,
        "(function_item name: (identifier) @title) @definition.function",
        None,
    )
    .err()
    .unwrap();
    assert!(err.contains("`title`"), "{err}");
}
