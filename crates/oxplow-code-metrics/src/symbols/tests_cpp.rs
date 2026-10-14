use super::tests::{expect, ids};

#[test]
fn cpp_definitions_and_their_ids() {
    let src = r#"
namespace geo { int area(const Shape& s) { return 0; } }
void f(int a) {}
void f(double x = 1.0) {}
class Widget { public: Widget(); ~Widget(); void draw() const; int size_; };
void Widget::draw() const {}
template <typename T> class Stack { void push(T v); };
struct P { int x; };
enum class Color { Red };
#define MAX 3
int counter = 0;
"#;
    assert_eq!(
        ids("a.cpp", src),
        expect(&[
            ("namespace", "geo"),
            ("function", "geo::area(const Shape&)"),
            ("function", "f(int)"),
            ("function", "f(double)"),
            ("class", "Widget"),
            ("constructor", "Widget::Widget()"),
            ("method", "Widget::~Widget()"),
            ("method", "Widget::draw()"),
            ("field", "Widget::size_"),
            ("method", "Widget::draw()~2"),
            ("class", "Stack"),
            ("method", "Stack::push(T)"),
            ("struct", "P"),
            ("field", "P::x"),
            ("enum", "Color"),
            ("enum_member", "Color::Red"),
            ("constant", "MAX"),
            ("variable", "counter"),
        ])
    );
}

#[test]
fn cpp_qualified_templates_operators_and_wrappers() {
    let src = r#"
namespace a::b { Foo& get(int n) { return x; } }
template <typename T> void Stack<T>::push(T v) {}
template <typename T> Stack<T>::Stack() {}
Widget::Widget(int n) : n_(n) {}
Widget::~Widget() {}
bool Widget::operator==(const Widget& o) const { return true; }
char **names(void) { return 0; }
using Id = int;
typedef struct { int x; } pt;
static const int limit = 3;
"#;
    assert_eq!(
        ids("a.cpp", src),
        expect(&[
            ("namespace", "a::b"),
            ("function", "a::b::get(int)"),
            ("method", "Stack::push(T)"),
            ("constructor", "Stack::Stack()"),
            ("constructor", "Widget::Widget(int)"),
            ("method", "Widget::~Widget()"),
            ("method", "Widget::operator==(const Widget&)"),
            ("function", "names(void)"),
            ("type", "Id"),
            ("type", "pt"),
            ("field", "pt::x"),
            ("variable", "limit"),
        ])
    );
}
