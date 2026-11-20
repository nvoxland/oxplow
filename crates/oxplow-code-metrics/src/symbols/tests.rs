use super::*;

#[test]
fn every_shipped_symbol_query_compiles_against_its_grammar() {
    for language in crate::for_tests::shipped().all() {
        if !language.analyzable() {
            continue;
        }
        let spec = language.check_symbols();
        assert!(spec.is_ok(), "{}: {}", language.name(), spec.err().unwrap());
    }
}

#[test]
fn a_definition_records_where_its_name_is_and_its_whole_extent() {
    let src = "struct A;\n\nimpl A {\n    pub fn go(&self) {\n        1;\n    }\n}\n";
    let go = extract(&crate::for_tests::shipped(), "a.rs", src)
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
    assert!(extract(&crate::for_tests::shipped(), "notes.txt", "fn a() {}").is_empty());
}

#[test]
fn a_query_capturing_an_unknown_name_is_refused() {
    let err = SymbolSpec::compile(
        &crate::for_tests::lang("rust").tree_sitter_language(),
        "(function_item name: (identifier) @name) @definition.lambda",
        None,
    )
    .err()
    .unwrap();
    assert!(err.contains("`definition.lambda`"), "{err}");
    let err = SymbolSpec::compile(
        &crate::for_tests::lang("rust").tree_sitter_language(),
        "(function_item name: (identifier) @title) @definition.function",
        None,
    )
    .err()
    .unwrap();
    assert!(err.contains("`title`"), "{err}");
}
