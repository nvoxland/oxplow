use super::*;

#[test]
fn rust_simple_function() {
    let src = r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    let f = &m[0];
    assert_eq!(f.name, "add");
    assert_eq!(f.parameter_count, 2);
    assert_eq!(f.complexity, 1);
    assert!(f.length >= 3);
}

#[test]
fn rust_branching_function_increments_complexity() {
    let src = r#"
fn classify(x: i32) -> &'static str {
    if x > 0 {
        "pos"
    } else if x < 0 {
        "neg"
    } else {
        "zero"
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    // base 1 + outer if + else_clause + inner if-as-expression
    assert!(m[0].complexity >= 3, "got {}", m[0].complexity);
}

#[test]
fn rust_match_increments_per_arm() {
    let src = r#"
fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        _ => "other",
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    // 3 match arms = +3
    assert_eq!(m[0].complexity, 4);
}

#[test]
fn typescript_arrow_function_with_branches() {
    let src = r#"
const handler = (req: Request, res: Response) => {
    if (req.method === "GET") {
        return res.send("ok");
    }
    if (req.method === "POST") {
        return res.send("created");
    }
    return res.send("?");
};
"#;
    let m = analyze_file("src/x.ts", src);
    assert!(!m.is_empty());
    let f = m.iter().find(|f| f.parameter_count == 2).expect("arrow");
    assert!(f.complexity >= 3, "got {}", f.complexity);
}

#[test]
fn python_function_with_loops() {
    let src = "
def histogram(items):
    out = {}
    for it in items:
        if it in out:
            out[it] += 1
        else:
            out[it] = 1
    return out
";
    let m = analyze_file("src/x.py", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].name, "histogram");
    assert_eq!(m[0].parameter_count, 1);
    assert!(m[0].complexity >= 3, "got {}", m[0].complexity);
}

#[test]
fn go_method_counts_receiver() {
    let src = r#"
package main

func (s *Server) handle(w http.ResponseWriter, r *http.Request) {
    if r.URL.Path == "/" {
        w.WriteHeader(200)
        return
    }
    w.WriteHeader(404)
}
"#;
    let m = analyze_file("main.go", src);
    let f = m.iter().find(|f| f.name == "handle").expect("method");
    assert_eq!(f.parameter_count, 2);
    assert!(f.complexity >= 2);
}

#[test]
fn java_method() {
    let src = r#"
class Foo {
    public int classify(int x) {
        if (x > 0) return 1;
        if (x < 0) return -1;
        return 0;
    }
}
"#;
    let m = analyze_file("Foo.java", src);
    let f = m.iter().find(|f| f.name == "classify").expect("method");
    assert_eq!(f.parameter_count, 1);
    assert!(f.complexity >= 3);
}

#[test]
fn unsupported_language_returns_empty() {
    let m = analyze_file("README.md", "# heading");
    assert!(m.is_empty());
}

#[test]
fn extensionless_path_is_not_classified() {
    // Regression: previously a path like "rs" (no extension) was
    // misclassified as Rust because `rsplit('.').next()` returned
    // the whole string.
    let m = analyze_file("rs", "fn x() {}");
    assert!(m.is_empty());
    let m = analyze_file("Makefile", "all: foo\n");
    assert!(m.is_empty());
}

#[test]
fn c_function_name_is_just_the_identifier() {
    let src = r#"
int classify(int x) {
    if (x > 0) return 1;
    if (x < 0) return -1;
    return 0;
}
"#;
    let m = analyze_file("src/x.c", src);
    let f = m.iter().find(|f| f.name == "classify").unwrap_or_else(|| {
        panic!(
            "expected name 'classify', got {:?}",
            m.iter().map(|f| &f.name).collect::<Vec<_>>()
        )
    });
    assert_eq!(f.parameter_count, 1);
    assert!(f.complexity >= 3);
}

#[test]
fn cpp_function_name_strips_declarator_decoration() {
    let src = r#"
int Foo::bar(int x, int y) {
    if (x > y) return x;
    return y;
}
"#;
    let m = analyze_file("src/x.cpp", src);
    let names: Vec<_> = m.iter().map(|f| f.name.as_str()).collect();
    // The qualified id parses as `Foo::bar`; the inner identifier is
    // `bar`. Either is acceptable as long as we don't return the
    // whole declarator including parameters.
    assert!(
        names.iter().any(|n| *n == "bar" || *n == "Foo::bar"),
        "got {:?}",
        names
    );
    assert!(
        !names.iter().any(|n| n.contains('(')),
        "name leaked declarator parens: {:?}",
        names
    );
}

#[test]
fn rust_else_chain_does_not_double_count() {
    // Regression: previously `if/else if/else` was scored as
    // `if + else + if + else` = 4. McCabe says +2 (the two ifs).
    let src = r#"
fn classify(x: i32) -> &'static str {
    if x > 0 {
        "pos"
    } else if x < 0 {
        "neg"
    } else {
        "zero"
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].complexity, 3, "got {}", m[0].complexity);
}

#[test]
fn rust_method_in_impl_records_container_path() {
    let src = r#"
struct Foo;
impl Foo {
    fn bar(&self) -> i32 { 1 }
}
"#;
    let m = analyze_file("src/x.rs", src);
    let bar = m.iter().find(|f| f.name == "bar").expect("method");
    assert_eq!(bar.container_path, vec!["Foo".to_string()]);
}

#[test]
fn rust_method_in_nested_module_records_full_container_path() {
    let src = r#"
mod outer {
    mod inner {
        struct Foo;
        impl Foo {
            fn bar(&self) -> i32 { 1 }
        }
    }
}
"#;
    let m = analyze_file("src/x.rs", src);
    let bar = m.iter().find(|f| f.name == "bar").expect("method");
    assert_eq!(
        bar.container_path,
        vec!["outer".to_string(), "inner".to_string(), "Foo".to_string()],
    );
}

#[test]
fn typescript_method_in_class_records_class_name() {
    let src = r#"
class Greeter {
    greet(name: string) {
        return "hi " + name;
    }
}
"#;
    let m = analyze_file("src/x.ts", src);
    let greet = m.iter().find(|f| f.name == "greet").expect("method");
    assert_eq!(greet.container_path, vec!["Greeter".to_string()]);
}

#[test]
fn python_method_in_class_records_class_name() {
    let src = "
class Greeter:
    def greet(self, name):
        return name
";
    let m = analyze_file("src/x.py", src);
    let greet = m.iter().find(|f| f.name == "greet").expect("method");
    assert_eq!(greet.container_path, vec!["Greeter".to_string()]);
}

#[test]
fn java_method_in_nested_class_records_full_container_path() {
    let src = r#"
class Outer {
    static class Inner {
        public int val() { return 1; }
    }
}
"#;
    let m = analyze_file("Outer.java", src);
    let val = m.iter().find(|f| f.name == "val").expect("method");
    assert_eq!(
        val.container_path,
        vec!["Outer".to_string(), "Inner".to_string()],
    );
}

#[test]
fn top_level_function_has_empty_container_path() {
    let src = "fn add(a: i32, b: i32) -> i32 { a + b }\n";
    let m = analyze_file("src/x.rs", src);
    assert_eq!(m.len(), 1);
    assert!(m[0].container_path.is_empty());
}

#[test]
fn rust_visibility_pub_vs_private() {
    let src = r#"
pub fn exported() {}
fn private_fn() {}
"#;
    let m = analyze_file("src/x.rs", src);
    let exported = m.iter().find(|f| f.name == "exported").unwrap();
    let priv_fn = m.iter().find(|f| f.name == "private_fn").unwrap();
    assert_eq!(exported.visibility, Visibility::Public);
    assert_eq!(priv_fn.visibility, Visibility::Private);
}

#[test]
fn typescript_class_method_visibility() {
    let src = r#"
class Greeter {
    private secret(): void {}
    public hello(): void {}
    plain(): void {}
}
"#;
    let m = analyze_file("src/x.ts", src);
    let secret = m.iter().find(|f| f.name == "secret").unwrap();
    let hello = m.iter().find(|f| f.name == "hello").unwrap();
    let plain = m.iter().find(|f| f.name == "plain").unwrap();
    assert_eq!(secret.visibility, Visibility::Private);
    assert_eq!(hello.visibility, Visibility::Public);
    // No accessibility modifier inside a class → public default.
    assert_eq!(plain.visibility, Visibility::Public);
}

#[test]
fn typescript_top_level_export_visibility() {
    let src = r#"
export function exported() {}
function helper() {}
"#;
    let m = analyze_file("src/x.ts", src);
    let exported = m.iter().find(|f| f.name == "exported").unwrap();
    let helper = m.iter().find(|f| f.name == "helper").unwrap();
    assert_eq!(exported.visibility, Visibility::Public);
    assert_eq!(helper.visibility, Visibility::Private);
}

#[test]
fn python_underscore_visibility() {
    let src = "
def _helper():
    pass
def public():
    pass
";
    let m = analyze_file("src/x.py", src);
    let helper = m.iter().find(|f| f.name == "_helper").unwrap();
    let public = m.iter().find(|f| f.name == "public").unwrap();
    assert_eq!(helper.visibility, Visibility::Private);
    assert_eq!(public.visibility, Visibility::Public);
}

#[test]
fn go_capitalization_visibility() {
    let src = r#"
package main
func Public() {}
func privateFn() {}
"#;
    let m = analyze_file("main.go", src);
    let pub_fn = m.iter().find(|f| f.name == "Public").unwrap();
    let priv_fn = m.iter().find(|f| f.name == "privateFn").unwrap();
    assert_eq!(pub_fn.visibility, Visibility::Public);
    assert_eq!(priv_fn.visibility, Visibility::Private);
}

#[test]
fn java_visibility_modifiers() {
    let src = r#"
class Foo {
    private int alpha() { return 0; }
    public int beta() { return 0; }
    int packagePriv() { return 0; }
}
"#;
    let m = analyze_file("Foo.java", src);
    let alpha = m.iter().find(|f| f.name == "alpha").unwrap();
    let beta = m.iter().find(|f| f.name == "beta").unwrap();
    let pkg = m.iter().find(|f| f.name == "packagePriv").unwrap();
    assert_eq!(alpha.visibility, Visibility::Private);
    assert_eq!(beta.visibility, Visibility::Public);
    // Package-private treated as Private (can't call from outside).
    assert_eq!(pkg.visibility, Visibility::Private);
}

#[test]
fn c_static_visibility() {
    let src = r#"
static int hidden(int x) { return x; }
int exported(int x) { return x; }
"#;
    let m = analyze_file("src/x.c", src);
    let hidden = m.iter().find(|f| f.name == "hidden").unwrap();
    let exported = m.iter().find(|f| f.name == "exported").unwrap();
    assert_eq!(hidden.visibility, Visibility::Private);
    assert_eq!(exported.visibility, Visibility::Public);
}

#[test]
fn nested_functions_each_get_their_own_record() {
    let src = r#"
fn outer() {
    fn inner(x: i32) -> i32 {
        if x > 0 { x } else { -x }
    }
    let _ = inner(1);
}
"#;
    let m = analyze_file("src/x.rs", src);
    assert_eq!(
        m.len(),
        2,
        "got {:?}",
        m.iter().map(|f| &f.name).collect::<Vec<_>>()
    );
    let inner = m.iter().find(|f| f.name == "inner").unwrap();
    let outer = m.iter().find(|f| f.name == "outer").unwrap();
    // Inner function's `if` should count toward inner, not outer.
    assert!(inner.complexity >= 2);
    assert_eq!(outer.complexity, 1);
}

#[test]
fn clojure_defn_simple() {
    let src = r#"
(ns demo.core)

(defn add [a b]
  (+ a b))
"#;
    let m = analyze_file("src/demo/core.clj", src);
    assert_eq!(m.len(), 1, "expected one function, got {m:#?}");
    let f = &m[0];
    assert_eq!(f.name, "add");
    assert_eq!(f.parameter_count, 2);
    assert_eq!(f.complexity, 1);
    assert_eq!(f.visibility, Visibility::Public);
}

#[test]
fn clojure_defn_minus_is_private() {
    let src = r#"
(defn- helper [x] x)
"#;
    let m = analyze_file("src/demo.clj", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].name, "helper");
    assert_eq!(m[0].visibility, Visibility::Private);
}

#[test]
fn clojure_metadata_private_marker_is_private() {
    let src = r#"
(defn ^:private secret [x] x)
"#;
    let m = analyze_file("src/demo.clj", src);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].name, "secret");
    assert_eq!(m[0].visibility, Visibility::Private);
}

#[test]
fn clojure_branching_increments_complexity() {
    let src = r#"
(defn classify [x]
  (cond
    (pos? x) :pos
    (neg? x) :neg
    :else :zero))
"#;
    let m = analyze_file("src/demo.clj", src);
    assert_eq!(m.len(), 1);
    assert!(m[0].complexity >= 2, "got {}", m[0].complexity);
}

#[test]
fn clojure_nested_fn_inside_defn_recorded_separately() {
    let src = r#"
(defn outer [xs]
  (map (fn [x] (* x 2)) xs))
"#;
    let m = analyze_file("src/demo.clj", src);
    assert_eq!(m.len(), 2, "got {m:#?}");
    let outer = m.iter().find(|f| f.name == "outer").unwrap();
    let inner = m.iter().find(|f| f.name != "outer").unwrap();
    assert_eq!(inner.name, "(anonymous)");
    assert_eq!(outer.parameter_count, 1);
}

#[test]
fn clojure_ignores_non_function_top_level_lists() {
    let src = r#"
(def some-data 42)
(when true (println "side effect"))
"#;
    let m = analyze_file("src/demo.clj", src);
    assert!(m.is_empty(), "expected no functions, got {m:#?}");
}

#[test]
fn markers_rust_line_and_block_comments() {
    let src = "// TODO: fix this\nfn a() {}\n/* FIXME: later\n   HACK: nested */\nfn b() {} // XXX cleanup\n";
    let m = markers(src, Language::Rust);
    let kinds: Vec<&str> = m.iter().map(|x| x.kind.as_str()).collect();
    assert_eq!(kinds, vec!["TODO", "FIXME", "HACK", "XXX"]);
    assert_eq!(m[0].line, 1);
    assert_eq!(m[0].text, "TODO: fix this");
    // Block-comment marker lines are offset within the comment.
    assert_eq!(m[1].line, 3);
    assert_eq!(m[2].line, 4);
    assert_eq!(m[3].line, 5);
}

#[test]
fn markers_only_inside_comments_not_string_literals() {
    // "TODO" in a string is not a marker; the comment one is.
    let src = "fn a() {\n    let s = \"TODO not a marker\";\n    // TODO real one\n}\n";
    let m = markers(src, Language::Rust);
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].line, 3);
}

#[test]
fn markers_word_boundary() {
    // TODONE / mastodon must not match TODO.
    let src = "// TODONE done, not a marker\n// see mastodon\n";
    assert!(markers(src, Language::Rust).is_empty());
}

#[test]
fn markers_clojure_semicolon_and_csharp() {
    let clj = "; TODO clojure marker\n(defn f [])\n";
    let cm = markers(clj, Language::Clojure);
    assert_eq!(cm.len(), 1);
    assert_eq!(cm[0].kind, "TODO");

    let cs = "// FIXME csharp\nclass C { }\n";
    let csm = markers(cs, Language::CSharp);
    assert_eq!(csm.len(), 1);
    assert_eq!(csm[0].kind, "FIXME");
}

#[test]
fn list_units_rust_functions_and_containers() {
    let src = r#"
mod inner {
    struct S;
    impl S {
        fn method(&self) {}
    }
}

fn top() {}
"#;
    let units = list_units("src/x.rs", src);
    let kinds: Vec<(&str, &str)> = units
        .iter()
        .map(|u| (u.kind.as_str(), u.name.as_str()))
        .collect();
    // mod → Module; impl → Class (named for its type); functions captured;
    // no Package unit (Rust doesn't declare one).
    assert!(kinds.contains(&("module", "inner")), "kinds: {kinds:?}");
    assert!(kinds.contains(&("class", "S")), "kinds: {kinds:?}");
    assert!(kinds.contains(&("function", "method")), "kinds: {kinds:?}");
    assert!(kinds.contains(&("function", "top")), "kinds: {kinds:?}");
    assert!(
        !units.iter().any(|u| u.kind == UnitKind::Package),
        "rust should not emit a package unit"
    );
    // The nested method records its containers.
    let method = units.iter().find(|u| u.name == "method").unwrap();
    assert_eq!(method.container_path, vec!["inner", "S"]);
}

#[test]
fn list_units_go_emits_package_from_directory() {
    let src = "package foo\n\nfunc Bar() {}\n";
    let units = list_units("pkg/foo/bar.go", src);
    // Go declares Package as a unit kind → the file's parent dir.
    let pkg = units
        .iter()
        .find(|u| u.kind == UnitKind::Package)
        .expect("go file should emit a package unit");
    assert_eq!(pkg.name, "pkg/foo");
    assert!(units
        .iter()
        .any(|u| u.kind == UnitKind::Function && u.name == "Bar"));
}

#[test]
fn list_units_empty_for_unsupported() {
    assert!(list_units("notes.txt", "TODO not code").is_empty());
}

#[test]
fn generated_source_detected_by_header_marker() {
    // The Tauri Specta / ts-rs / protobuf convention: a do-not-edit header.
    let specta = "// This file has been generated by Tauri Specta. Do not edit this file manually.\n\nexport const x = 1;\n";
    assert!(is_generated_source("src/bindings.ts", specta));
    let go = "// Code generated by protoc-gen-go. DO NOT EDIT.\npackage pb\n";
    assert!(is_generated_source("pb/thing.go", go));
    let meta = "/* @generated */\nconst a = 1;\n";
    assert!(is_generated_source("src/a.js", meta));
}

#[test]
fn generated_source_detected_by_path() {
    assert!(is_generated_source(
        "apps/desktop/src/tauri-bridge/generated/bindings.ts",
        "export const x = 1;\n"
    ));
    assert!(is_generated_source("src/api.generated.ts", "export {};\n"));
}

#[test]
fn generated_marker_only_counts_in_the_header() {
    // "do not edit" deep in a hand-written file (say, inside a string or a
    // late comment) must not flag the whole file as generated.
    let mut src = String::from("fn main() {\n");
    for _ in 0..20 {
        src.push_str("    println!(\"x\");\n");
    }
    src.push_str("    // do not edit this constant lightly\n}\n");
    assert!(!is_generated_source("src/main.rs", &src));
}

#[test]
fn ordinary_source_is_not_generated() {
    assert!(!is_generated_source(
        "src/lib.rs",
        "//! A library.\nfn work() {}\n"
    ));
    // A path merely *containing* the substring shouldn't trip the segment check.
    assert!(!is_generated_source(
        "src/degenerated_cases.rs",
        "fn edge() {}\n"
    ));
}
