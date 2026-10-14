; What a Java file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-java's tags.scm; adds fields, enum constants
; and records. Callables carry their parameter types (Java overloads).

(class_declaration name: (identifier) @name) @definition.class
(record_declaration name: (identifier) @name) @definition.class
(interface_declaration name: (identifier) @name) @definition.interface
(annotation_type_declaration name: (identifier) @name) @definition.interface
(enum_declaration name: (identifier) @name) @definition.enum
(enum_constant name: (identifier) @name) @definition.enum_member
(method_declaration name: (identifier) @name parameters: (formal_parameters) @parameters) @definition.method
(constructor_declaration name: (identifier) @name parameters: (formal_parameters) @parameters) @definition.constructor
(field_declaration declarator: (variable_declarator name: (identifier) @name)) @definition.field
(constant_declaration declarator: (variable_declarator name: (identifier) @name)) @definition.constant
