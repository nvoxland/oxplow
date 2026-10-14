use super::tests::{expect, ids};

#[test]
fn typescript_functions_and_arrow_bindings() {
    let src = r#"
export function parseRef(s: string) {}
const handler = () => {};
export const g = (a: number) => a;
const f = function () {};
function* gen() {}
function outer() { const inner = () => {}; }
items.map(x => x + 1);
describe("thing", () => { it("works", () => {}); });
"#;
    assert_eq!(
        ids("a.ts", src),
        expect(&[
            ("function", "parseRef"),
            ("function", "handler"),
            ("function", "g"),
            ("function", "f"),
            ("function", "gen"),
            ("function", "outer"),
            ("function", "outer::inner"),
        ])
    );
}

#[test]
fn typescript_class_members() {
    let src = r#"
class TabStore {
    open() {}
    private x = 1;
    static y: number;
    constructor() {}
    onClick = () => {};
    get size() { return 1; }
    #secret = 2;
    abstract run(): void;
}
abstract class Base { abstract go(): void; }
"#;
    assert_eq!(
        ids("a.ts", src),
        expect(&[
            ("class", "TabStore"),
            ("method", "TabStore::open"),
            ("field", "TabStore::x"),
            ("field", "TabStore::y"),
            ("constructor", "TabStore::constructor"),
            ("method", "TabStore::onClick"),
            ("method", "TabStore::size"),
            ("field", "TabStore::#secret"),
            ("method", "TabStore::run"),
            ("class", "Base"),
            ("method", "Base::go"),
        ])
    );
}

#[test]
fn typescript_interfaces_types_enums_namespaces() {
    let src = r#"
interface TabState { activeId: string; close(): void }
type Role = "a" | "b";
enum Mode { A, B = 2 }
namespace NS { export function f() {} }
declare module "m" { }
module M {}
"#;
    assert_eq!(
        ids("a.ts", src),
        expect(&[
            ("interface", "TabState"),
            ("field", "TabState::activeId"),
            ("method", "TabState::close"),
            ("type", "Role"),
            ("enum", "Mode"),
            ("enum_member", "Mode::A"),
            ("enum_member", "Mode::B"),
            ("namespace", "NS"),
            ("function", "NS::f"),
            ("module", "M"),
        ])
    );
}

#[test]
fn typescript_top_level_values_only() {
    let src = r#"
const X = 3;
let y;
var z = 1;
export const W = { a: 1 };
function f() { const local = 1; let l2 = 2; }
"#;
    assert_eq!(
        ids("a.ts", src),
        expect(&[
            ("constant", "X"),
            ("variable", "y"),
            ("variable", "z"),
            ("constant", "W"),
            ("function", "f"),
        ])
    );
}

#[test]
fn typescript_overloads_are_one_symbol() {
    let src = r#"
function format(a: string): string;
function format(a: number): string;
function format(a: any): string { return ""; }
"#;
    assert_eq!(ids("a.ts", src), expect(&[("function", "format")]));
}

#[test]
fn tsx_component() {
    let src = "export function App() { return <div/>; }\nconst B = () => <span/>;\n";
    assert_eq!(
        ids("a.tsx", src),
        expect(&[("function", "App"), ("function", "B")])
    );
}
