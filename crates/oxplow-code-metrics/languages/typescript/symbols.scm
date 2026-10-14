; What a TypeScript/TSX file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter's tags.scm; adds fields, enum members, bound
; functions and top-level constants/variables.

; Functions: declarations, and a function/arrow bound to a name anywhere.
; The definition is the function node so its parameters are counted;
; callbacks and other anonymous functions are not symbols.
(function_declaration name: (identifier) @name) @definition.function
(generator_function_declaration name: (identifier) @name) @definition.function
(variable_declarator
  name: (identifier) @name
  value: [(arrow_function) (function_expression) (generator_function)] @definition.function)

; Classes and their members. A `constructor` method is a constructor.
(class_declaration name: (type_identifier) @name) @definition.class
(abstract_class_declaration name: (type_identifier) @name) @definition.class
(method_definition name: (_) @name
  (#not-eq? @name "constructor")) @definition.method
(method_definition name: (_) @name
  (#eq? @name "constructor")) @definition.constructor
(public_field_definition name: (_) @name
  value: [(arrow_function) (function_expression) (generator_function)]) @definition.method
(public_field_definition name: (_) @name
  !value) @definition.field
(public_field_definition name: (_) @name
  value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.field

; Interface members.
(interface_declaration name: (type_identifier) @name) @definition.interface
(property_signature name: (_) @name) @definition.field
(method_signature name: (_) @name) @definition.method
(abstract_method_signature name: (_) @name) @definition.method

; Overload signatures are not symbols: the implementation is the one.
; (function_signature is deliberately not captured.)

(type_alias_declaration name: (type_identifier) @name) @definition.type
(enum_declaration name: (identifier) @name) @definition.enum
(enum_body name: (_) @name @definition.enum_member)
(enum_assignment name: (_) @name) @definition.enum_member
(internal_module name: (_) @name) @definition.namespace
(module name: (identifier) @name) @definition.module

; Top-level values (not functions; locals are not symbols).
(program
  (lexical_declaration "const" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.constant))
(program
  (lexical_declaration "let" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.variable))
(program
  (lexical_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(program
  (variable_declaration (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.variable))
(program
  (variable_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(export_statement
  (lexical_declaration "const" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.constant))
(export_statement
  (lexical_declaration "let" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.variable))
(export_statement
  (lexical_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(export_statement
  (variable_declaration (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this) (as_expression) (satisfies_expression) (non_null_expression)]) @definition.variable))
(export_statement
  (variable_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
