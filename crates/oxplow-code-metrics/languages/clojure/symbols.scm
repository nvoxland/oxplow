; What a Clojure file defines (`.context/language-support.md` "Symbols").
; Every form is a `list_lit`, so definitions are found by their head
; symbol. Closures (`fn`) and `let` are not symbols; their decisions
; count toward the enclosing definition. Metadata (`^:private`) and
; docstrings sit between the parts, so name patterns anchor only on
; the head and the name.

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name)
  (#any-of? @_h "defn" "defn-" "definline" "defmulti")) @definition.function

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name)
  (#eq? @_h "defmacro")) @definition.macro

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name) . (_) @signature
  (#eq? @_h "defmethod")) @definition.function

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name)
  (#eq? @_h "ns")) @definition.module

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name)
  (#any-of? @_h "defrecord" "deftype")) @definition.struct

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name)
  (#eq? @_h "defprotocol")) @definition.interface

; A protocol's method signatures: `(area [this])`.
(list_lit . (sym_lit (sym_name) @_h) . (sym_lit)
  (list_lit . (sym_lit (sym_name) @name) . (vec_lit)) @definition.method
  (#eq? @_h "defprotocol"))

; `(def x (fn …))` is a function whose extent is the `fn`; any other
; value makes a variable (the `fn` check is on the value's text, so the
; two patterns never both match one `def`).
(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name) . (list_lit . (sym_lit (sym_name) @_f)) @definition.function
  (#any-of? @_h "def" "defonce")
  (#any-of? @_f "fn" "fn*"))

(list_lit . (sym_lit (sym_name) @_h) . (sym_lit (sym_name) @name) . (_)? @_v
  (#any-of? @_h "def" "defonce")
  (#not-match? @_v "^\\(\\s*fn\\*?[\\s\\[(]")) @definition.variable
