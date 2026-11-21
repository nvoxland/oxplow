; What an Elixir file defines.
; A definition is a `call` whose target is `defmodule`, `def`, `defp`, ...
; and whose first argument holds the name. A function head is `name(args)`,
; `name(args) when guard` or a bare `name`; a module is an alias.

(call
  target: (identifier) @_h
  (arguments (alias) @name)
  (#eq? @_h "defmodule")) @definition.module

; def name(args) do ... end / def name(args), do: ...
(call
  target: (identifier) @_h
  (arguments (call target: (identifier) @name))
  (#any-of? @_h "def" "defp")) @definition.function

; def name do ... end
(call
  target: (identifier) @_h
  (arguments . (identifier) @name)
  (#any-of? @_h "def" "defp")) @definition.function

; def name(args) when guard
(call
  target: (identifier) @_h
  (arguments (binary_operator left: (call target: (identifier) @name) operator: "when"))
  (#any-of? @_h "def" "defp")) @definition.function

(call
  target: (identifier) @_h
  (arguments (call target: (identifier) @name))
  (#any-of? @_h "defmacro" "defmacrop")) @definition.macro

(call
  target: (identifier) @_h
  (arguments (alias) @name)
  (#eq? @_h "defprotocol")) @definition.interface

(call
  target: (identifier) @_h
  (arguments (alias) @name)
  (#eq? @_h "defimpl")) @definition.class
