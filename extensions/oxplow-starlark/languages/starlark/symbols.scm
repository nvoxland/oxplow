; What a Starlark file defines: top-level functions, and top-level
; assignments to a name (constants and variables).

(function_definition name: (identifier) @name) @definition.function
(module (expression_statement (assignment left: (identifier) @name)) @definition.variable)
