use super::tests::{expect, ids};

#[test]
fn csharp_namespace_types_and_members() {
    let src = r#"
namespace Acme {
    class P {
        void Run() {}
        public int Size { get; set; }
        private int _x;
        const int Max = 3;
        public event EventHandler Changed;
        P(int x) {}
        ~P() {}
    }
}
"#;
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("namespace", "Acme"),
            ("class", "Acme::P"),
            ("method", "Acme::P::Run()"),
            ("field", "Acme::P::Size"),
            ("field", "Acme::P::_x"),
            ("constant", "Acme::P::Max"),
            ("field", "Acme::P::Changed"),
            ("constructor", "Acme::P::P(int)"),
            ("method", "Acme::P::~P()"),
        ])
    );
}

#[test]
fn csharp_overloads_drop_names_and_defaults() {
    let src = r#"
class P {
    int Parse(string s, int n = 0) => n;
    int Parse(List<string> xs) => 0;
}
"#;
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("class", "P"),
            ("method", "P::Parse(string,int)"),
            ("method", "P::Parse(List<string>)"),
        ])
    );
}

#[test]
fn csharp_type_kinds() {
    let src = r#"
interface IShape { double Area(); }
struct Pt { }
record R(int X);
enum Color { Red, Green }
delegate void Handler(int x);
"#;
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("interface", "IShape"),
            ("method", "IShape::Area()"),
            ("struct", "Pt"),
            ("class", "R"),
            ("enum", "Color"),
            ("enum_member", "Color::Red"),
            ("enum_member", "Color::Green"),
            ("type", "Handler"),
        ])
    );
}

#[test]
fn csharp_local_functions_nest_and_lambdas_do_not() {
    let src = r#"
class P {
    void Run() {
        int local(int a) { return a; }
        Func<int,int> f = x => x + 1;
    }
}
"#;
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("class", "P"),
            ("method", "P::Run()"),
            ("function", "P::Run::local(int)"),
        ])
    );
}

#[test]
fn csharp_file_scoped_namespace() {
    let src = "namespace Acme;\nclass P { void Run() {} }\n";
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("namespace", "Acme"),
            ("class", "P"),
            ("method", "P::Run()"),
        ])
    );
}

#[test]
fn csharp_multiple_declarators_and_const() {
    let src = "class P { public static readonly int A = 1, B = 2; public const int C = 3, D = 4; }";
    assert_eq!(
        ids("a.cs", src),
        expect(&[
            ("class", "P"),
            ("field", "P::A"),
            ("field", "P::B"),
            ("constant", "P::C"),
            ("constant", "P::D"),
        ])
    );
}
