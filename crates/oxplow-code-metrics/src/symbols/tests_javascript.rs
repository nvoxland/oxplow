use super::tests::{expect, ids};

#[test]
fn javascript_functions_and_arrow_bindings() {
    let src = r#"
export function parseRef(s) {}
const handler = () => {};
const f = function () {};
function* gen() {}
function outer() { const inner = () => {}; }
items.map(x => x + 1);
describe("thing", () => { it("works", () => {}); });
"#;
    assert_eq!(
        ids("a.js", src),
        expect(&[
            ("function", "parseRef"),
            ("function", "handler"),
            ("function", "f"),
            ("function", "gen"),
            ("function", "outer"),
            ("function", "outer::inner"),
        ])
    );
}

#[test]
fn javascript_class_members() {
    let src = r#"
class TabStore {
    open() {}
    x = 1;
    static y;
    constructor() {}
    onClick = () => {};
    get size() { return 1; }
    #secret = 2;
}
"#;
    assert_eq!(
        ids("a.js", src),
        expect(&[
            ("class", "TabStore"),
            ("method", "TabStore::open"),
            ("field", "TabStore::x"),
            ("field", "TabStore::y"),
            ("constructor", "TabStore::constructor"),
            ("method", "TabStore::onClick"),
            ("method", "TabStore::size"),
            ("field", "TabStore::#secret"),
        ])
    );
}

#[test]
fn javascript_top_level_values_only() {
    let src = r#"
const X = 3;
let y;
var z = 1;
export const W = { a: 1 };
function f() { const local = 1; let l2 = 2; }
"#;
    assert_eq!(
        ids("a.jsx", src),
        expect(&[
            ("constant", "X"),
            ("variable", "y"),
            ("variable", "z"),
            ("constant", "W"),
            ("function", "f"),
        ])
    );
}
