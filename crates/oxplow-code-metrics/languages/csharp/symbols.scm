; What a C# file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-c-sharp's tags.scm; adds types, members and
; local functions. Callables carry their parameter types (overloads).
; Lambdas are not symbols.

(namespace_declaration name: (_) @name) @definition.namespace
(file_scoped_namespace_declaration name: (_) @name) @definition.namespace

(class_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
(struct_declaration name: (identifier) @name) @definition.struct
(interface_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(enum_member_declaration name: (identifier) @name) @definition.enum_member
(delegate_declaration name: (identifier) @name) @definition.type

(method_declaration name: (identifier) @name parameters: (parameter_list) @parameters) @definition.method
(constructor_declaration name: (identifier) @name parameters: (parameter_list) @parameters) @definition.constructor
(destructor_declaration "~" @name name: (identifier) @name parameters: (parameter_list) @parameters) @definition.method
(operator_declaration operator: _ @name parameters: (parameter_list) @parameters) @definition.method
(indexer_declaration "this" @name parameters: (bracketed_parameter_list) @parameters) @definition.method
(local_function_statement name: (identifier) @name parameters: (parameter_list) @parameters) @definition.function

(property_declaration name: (identifier) @name) @definition.field
(event_declaration name: (identifier) @name) @definition.field
(field_declaration
  (modifier) @_m
  (variable_declaration (variable_declarator name: (identifier) @name) @definition.constant)
  (#eq? @_m "const"))
(field_declaration
  (variable_declaration (variable_declarator name: (identifier) @name) @definition.field))
(event_field_declaration
  (variable_declaration (variable_declarator name: (identifier) @name) @definition.field))
