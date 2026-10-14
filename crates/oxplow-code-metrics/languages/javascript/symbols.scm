; What a JavaScript file defines (`.context/language-support.md` "Symbols").
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
(class_declaration name: (identifier) @name) @definition.class
(method_definition name: (_) @name
  (#not-eq? @name "constructor")) @definition.method
(method_definition name: (_) @name
  (#eq? @name "constructor")) @definition.constructor
(field_definition property: (_) @name
  value: [(arrow_function) (function_expression) (generator_function)]) @definition.method
(field_definition property: (_) @name
  !value) @definition.field
(field_definition property: (_) @name
  value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.field

; Top-level values (not functions; locals are not symbols).
(program
  (lexical_declaration "const" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.constant))
(program
  (lexical_declaration "let" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.variable))
(program
  (lexical_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(program
  (variable_declaration (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.variable))
(program
  (variable_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(export_statement
  (lexical_declaration "const" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.constant))
(export_statement
  (lexical_declaration "let" (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.variable))
(export_statement
  (lexical_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
(export_statement
  (variable_declaration (variable_declarator name: (identifier) @name value: [(number) (string) (template_string) (true) (false) (null) (undefined) (identifier)
     (object) (array) (call_expression) (new_expression) (binary_expression)
     (unary_expression) (member_expression) (subscript_expression) (await_expression)
     (ternary_expression) (regex) (parenthesized_expression) (class) (update_expression)
     (assignment_expression) (sequence_expression) (meta_property) (this)]) @definition.variable))
(export_statement
  (variable_declaration (variable_declarator name: (identifier) @name !value) @definition.variable))
