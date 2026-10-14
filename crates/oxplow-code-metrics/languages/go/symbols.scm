; What a Go file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-go's tags.scm. Go has no overloading, so no
; signatures. A method names its receiver type through `@scope`
; (pointer and generic receivers included). Func literals aren't symbols.
; A name list (`a, b int`, `const (X = 1)`) defines each name, so those
; patterns take the identifier itself as the definition.

(function_declaration name: (identifier) @name) @definition.function

(method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @scope
        (pointer_type (type_identifier) @scope)
        (generic_type type: (type_identifier) @scope)
        (pointer_type (generic_type type: (type_identifier) @scope))
      ]))
  name: (field_identifier) @name) @definition.function

(type_spec name: (type_identifier) @name type: (struct_type)) @definition.struct
(type_spec name: (type_identifier) @name type: (interface_type)) @definition.interface
(type_spec name: (type_identifier) @name type: [
  (type_identifier) (qualified_type) (pointer_type) (slice_type) (array_type)
  (map_type) (channel_type) (function_type) (generic_type) (parenthesized_type)
]) @definition.type
(type_alias name: (type_identifier) @name) @definition.type

(field_declaration name: (field_identifier) @name @definition.field)
(method_elem name: (field_identifier) @name) @definition.function

(source_file
  (const_declaration (const_spec name: (identifier) @name @definition.constant)))
(source_file
  (var_declaration (var_spec name: (identifier) @name @definition.variable)))
(source_file
  (var_declaration (var_spec_list (var_spec name: (identifier) @name @definition.variable))))
