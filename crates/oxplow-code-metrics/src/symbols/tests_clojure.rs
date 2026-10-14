use super::tests::{expect, ids};

#[test]
fn clojure_defs_and_their_ids() {
    let src = r#"
(ns acme.core
  (:require [clojure.string :as str]))

(defn parse [x] x)
(defn- helper [] 1)
(defn ^:private secret
  "A docstring."
  [x] x)
(defn documented
  "Docs"
  {:added "1.0"}
  [x] x)
(def timeout 30)
(def handler (fn [req] req))
(def ^:private hidden 1)
(defonce state (atom {}))
(def bare)
(defmacro unless [c & body] `(if ~c nil (do ~@body)))
(let [f (fn [x] x)] (f 1))
(map (fn [x] x) [1])
"#;
    assert_eq!(
        ids("a.clj", src),
        expect(&[
            ("module", "acme.core"),
            ("function", "parse"),
            ("function", "helper"),
            ("function", "secret"),
            ("function", "documented"),
            ("variable", "timeout"),
            ("function", "handler"),
            ("variable", "hidden"),
            ("variable", "state"),
            ("variable", "bare"),
            ("macro", "unless"),
        ])
    );
}

#[test]
fn clojure_multimethods_name_their_dispatch_value() {
    let src = r#"
(defmulti area :shape)
(defmethod area :circle [s] 1)
(defmethod area :square [s] 2)
(defmethod area [:a :b] [s] 3)
"#;
    assert_eq!(
        ids("a.clj", src),
        expect(&[
            ("function", "area"),
            ("function", "area(:circle)"),
            ("function", "area(:square)"),
            ("function", "area([:a :b])"),
        ])
    );
}

#[test]
fn clojure_protocols_and_records() {
    let src = r#"
(defprotocol Shape
  (area [this])
  (perimeter [this] "doc"))
(defrecord Point [x y])
(deftype Box [w])
"#;
    assert_eq!(
        ids("a.clj", src),
        expect(&[
            ("interface", "Shape"),
            ("method", "Shape::area"),
            ("method", "Shape::perimeter"),
            ("struct", "Point"),
            ("struct", "Box"),
        ])
    );
}

#[test]
fn clojure_def_of_fn_is_the_function_and_the_fn_is_not_a_second_symbol() {
    let src = "(def handler (fn handler* [req] (if req 1 2)))";
    assert_eq!(ids("a.cljs", src), expect(&[("function", "handler")]));
}
