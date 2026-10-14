use super::tests::{expect, ids};

#[test]
fn c_definitions_and_their_ids() {
    let src = r#"
#define MAX 3
#define SQ(x) ((x)*(x))
struct config { int depth; char *name; };
union u { int a; float b; };
enum color { RED, GREEN };
typedef struct { int x; int y; } point;
typedef int myint;
int counter = 0;
int proto(int a);
static int parse_args(int argc, char **argv) { return 0; }
"#;
    assert_eq!(
        ids("a.c", src),
        expect(&[
            ("constant", "MAX"),
            ("macro", "SQ"),
            ("struct", "config"),
            ("field", "config::depth"),
            ("field", "config::name"),
            ("struct", "u"),
            ("field", "u::a"),
            ("field", "u::b"),
            ("enum", "color"),
            ("enum_member", "color::RED"),
            ("enum_member", "color::GREEN"),
            ("type", "point"),
            ("field", "point::x"),
            ("field", "point::y"),
            ("type", "myint"),
            ("variable", "counter"),
            ("function", "parse_args"),
        ])
    );
}

#[test]
fn c_typedef_forms_pointers_and_prototypes() {
    let src = r#"
typedef enum { A, B } mode;
typedef struct node { int v; struct node *next; void (*cb)(int); int arr[3]; } node_t;
typedef int (*handler)(int);
extern int shared, other;
int proto(void);
char **make(void) { int local = 1; return 0; }
"#;
    assert_eq!(
        ids("a.c", src),
        expect(&[
            ("enum", "mode"),
            ("enum_member", "mode::A"),
            ("enum_member", "mode::B"),
            ("struct", "node"),
            ("field", "node::v"),
            ("field", "node::next"),
            ("field", "node::cb"),
            ("field", "node::arr"),
            ("type", "node_t"),
            ("type", "handler"),
            ("variable", "shared"),
            ("variable", "other"),
            ("function", "make"),
        ])
    );
}
