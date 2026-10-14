; What a C file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-c's tags.scm. Only function definitions (not
; prototypes). C has no overloading, so no signatures. A typedef of an
; anonymous struct/union/enum is the type, and the members nest under it;
; a typedef of a named type is just the alias name (the members nest under
; the named type). `a, b` declarator lists define each name, so those
; patterns take the identifier itself as the definition.

; Functions, through up to two pointer levels (`char **f(void)`).
(function_definition
  declarator: (function_declarator declarator: (identifier) @name)) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (function_declarator declarator: (identifier) @name))) @definition.function
(function_definition
  declarator: (pointer_declarator
    declarator: (pointer_declarator
      declarator: (function_declarator declarator: (identifier) @name)))) @definition.function

; Types.
(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(enumerator name: (identifier) @name) @definition.enum_member

; `typedef struct { … } point;` — the typedef is the type and holds the members.
(type_definition
  type: [(struct_specifier !name body: (_)) (union_specifier !name body: (_))]
  declarator: (type_identifier) @name) @definition.type
(type_definition
  type: (enum_specifier !name body: (_))
  declarator: (type_identifier) @name) @definition.enum
; Any other typedef: just the alias name.
(type_definition
  type: [
    (primitive_type) (sized_type_specifier) (type_identifier) (macro_type_specifier)
    (struct_specifier name: (_)) (union_specifier name: (_)) (enum_specifier name: (_))
  ]
  declarator: (type_identifier) @name @definition.type)
(type_definition
  declarator: [
    (pointer_declarator declarator: (type_identifier) @name @definition.type)
    (function_declarator declarator: (parenthesized_declarator
      (pointer_declarator declarator: (type_identifier) @name @definition.type)))
  ])

; Fields.
(field_declaration declarator: (field_identifier) @name @definition.field)
(field_declaration declarator: (pointer_declarator declarator: (field_identifier) @name @definition.field))
(field_declaration declarator: (pointer_declarator declarator: (pointer_declarator declarator: (field_identifier) @name @definition.field)))
(field_declaration declarator: (array_declarator declarator: (field_identifier) @name @definition.field))
(field_declaration declarator: (function_declarator
  declarator: (parenthesized_declarator (pointer_declarator declarator: (field_identifier) @name @definition.field))))

; Macros.
(preproc_def name: (identifier) @name value: (preproc_arg)) @definition.constant
(preproc_function_def name: (identifier) @name) @definition.macro

; File-scope variables.
(translation_unit
  (declaration declarator: (init_declarator declarator: (identifier) @name @definition.variable)))
(translation_unit
  (declaration declarator: (identifier) @name @definition.variable))
(translation_unit
  (declaration declarator: (pointer_declarator declarator: (identifier) @name @definition.variable)))
(translation_unit
  (declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @name @definition.variable))))
(translation_unit
  (declaration declarator: (array_declarator declarator: (identifier) @name @definition.variable)))
