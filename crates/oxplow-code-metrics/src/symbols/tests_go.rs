use super::tests::{expect, ids};

#[test]
fn go_definitions_and_their_ids() {
    let src = r#"
package main

type Server struct { addr string; Port int }
type Shape interface { Area() float64 }
type ID int
type Box[T any] struct { v T }
const X = 1
var y int

func Main() { f := func() {}; f() }
func (s *Server) Start() {}
func (s Server) Stop() {}
func (b *Box[T]) Get() T { return b.v }
"#;
    assert_eq!(
        ids("main.go", src),
        expect(&[
            ("struct", "Server"),
            ("field", "Server::addr"),
            ("field", "Server::Port"),
            ("interface", "Shape"),
            ("method", "Shape::Area"),
            ("type", "ID"),
            ("struct", "Box"),
            ("field", "Box::v"),
            ("constant", "X"),
            ("variable", "y"),
            ("function", "Main"),
            ("method", "Server::Start"),
            ("method", "Server::Stop"),
            ("method", "Box::Get"),
        ])
    );
}

#[test]
fn go_grouped_names_and_locals() {
    let src = r#"
package p

const (
	A = iota
	B
)
var (
	m, n int
)
type Alias = string

func f() { var local int; const c = 1; _ = func() {} }
"#;
    assert_eq!(
        ids("p.go", src),
        expect(&[
            ("constant", "A"),
            ("constant", "B"),
            ("variable", "m"),
            ("variable", "n"),
            ("type", "Alias"),
            ("function", "f"),
        ])
    );
}
