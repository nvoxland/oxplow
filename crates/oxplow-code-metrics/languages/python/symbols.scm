; What a Python file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-python's tags.scm; adds module variables and
; class attributes. A `def` inside a class is a method.

(class_definition name: (identifier) @name) @definition.class
(function_definition name: (identifier) @name) @definition.function
(module (expression_statement (assignment left: (identifier) @name)) @definition.variable)
(class_definition
  body: (block (expression_statement (assignment left: (identifier) @name)) @definition.field))
