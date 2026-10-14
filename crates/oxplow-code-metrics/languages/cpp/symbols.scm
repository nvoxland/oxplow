; What a C++ file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-cpp's tags.scm, plus everything the C query has.
; Every callable carries its parameter types as its signature (overloads),
; defaults dropped. A `::` in a name (`geo::area`,
; `Widget::draw`) puts the leading parts in the container. A callable
; named like its class (in-class) or its qualifier (out-of-line) is a
; constructor; a destructor is a method (it tears down, it doesn't
; construct). A member declaration (`void draw() const;`) is a method; its
; out-of-line definition in the same file is `~2`.
; The callable patterns repeat per declarator wrapper (`T *f()`, `T &f()`,
; `T **f()`), because a query can't recurse.

; Constructors, first so they win over the plain function patterns.
(class_specifier name: (type_identifier) @_c
  body: (field_declaration_list (function_definition declarator: (function_declarator declarator: (identifier) @name parameters: (parameter_list) @parameters)) @definition.constructor)
  (#eq? @_c @name))
(class_specifier name: (type_identifier) @_c
  body: (field_declaration_list (field_declaration declarator: (function_declarator declarator: (identifier) @name parameters: (parameter_list) @parameters)) @definition.constructor)
  (#eq? @_c @name))
(class_specifier name: (type_identifier) @_c
  body: (field_declaration_list (declaration declarator: (function_declarator declarator: (identifier) @name parameters: (parameter_list) @parameters)) @definition.constructor)
  (#eq? @_c @name))
((function_definition declarator: (function_declarator declarator: (qualified_identifier scope: (_) @_s @scope name: (identifier) @name) parameters: (parameter_list) @parameters)) @definition.constructor
 (#eq? @_s @name))
((function_definition declarator: (function_declarator declarator: (qualified_identifier scope: (template_type name: (type_identifier) @_s @scope) name: (identifier) @name) parameters: (parameter_list) @parameters)) @definition.constructor
 (#eq? @_s @name))

; Functions and methods. A template-qualified scope (`Stack<T>::push`) goes first so its container is `Stack`.
(function_definition declarator: (function_declarator declarator: (qualified_identifier scope: (template_type name: (type_identifier) @scope) name: (_) @name) parameters: (parameter_list) @parameters)) @definition.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: (qualified_identifier scope: (template_type name: (type_identifier) @scope) name: (_) @name) parameters: (parameter_list) @parameters))) @definition.function
(function_definition declarator: (reference_declarator (function_declarator declarator: (qualified_identifier scope: (template_type name: (type_identifier) @scope) name: (_) @name) parameters: (parameter_list) @parameters))) @definition.function
(function_definition declarator: (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: (qualified_identifier scope: (template_type name: (type_identifier) @scope) name: (_) @name) parameters: (parameter_list) @parameters)))) @definition.function
(function_definition declarator: (function_declarator declarator: [(identifier) (field_identifier) (operator_name) (destructor_name) (qualified_identifier scope: (_) name: (_))] @name parameters: (parameter_list) @parameters)) @definition.function
(function_definition declarator: (pointer_declarator declarator: (function_declarator declarator: [(identifier) (field_identifier) (operator_name) (destructor_name) (qualified_identifier scope: (_) name: (_))] @name parameters: (parameter_list) @parameters))) @definition.function
(function_definition declarator: (reference_declarator (function_declarator declarator: [(identifier) (field_identifier) (operator_name) (destructor_name) (qualified_identifier scope: (_) name: (_))] @name parameters: (parameter_list) @parameters))) @definition.function
(function_definition declarator: (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: [(identifier) (field_identifier) (operator_name) (destructor_name) (qualified_identifier scope: (_) name: (_))] @name parameters: (parameter_list) @parameters)))) @definition.function

; Member declarations (`void draw() const;`): a function directly inside a class is a method.
(field_declaration_list (field_declaration declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters)) @definition.function)
(field_declaration_list (field_declaration declarator: (pointer_declarator declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters))) @definition.function)
(field_declaration_list (field_declaration declarator: (reference_declarator (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters))) @definition.function)
(field_declaration_list (field_declaration declarator: (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters)))) @definition.function)
(field_declaration_list (declaration declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters)) @definition.function)
(field_declaration_list (declaration declarator: (pointer_declarator declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters))) @definition.function)
(field_declaration_list (declaration declarator: (reference_declarator (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters))) @definition.function)
(field_declaration_list (declaration declarator: (pointer_declarator declarator: (pointer_declarator declarator: (function_declarator declarator: [(field_identifier) (identifier) (operator_name) (destructor_name)] @name parameters: (parameter_list) @parameters)))) @definition.function)

; Types.
(class_specifier name: (type_identifier) @name body: (_)) @definition.class
(class_specifier name: (template_type name: (type_identifier) @name) body: (_)) @definition.class
(struct_specifier name: (type_identifier) @name body: (_)) @definition.struct
(struct_specifier name: (template_type name: (type_identifier) @name) body: (_)) @definition.struct
(union_specifier name: (type_identifier) @name body: (_)) @definition.struct
(enum_specifier name: (type_identifier) @name body: (_)) @definition.enum
(enumerator name: (identifier) @name) @definition.enum_member
(namespace_definition name: (_) @name) @definition.namespace
(alias_declaration name: (type_identifier) @name) @definition.type

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
    (primitive_type) (sized_type_specifier) (type_identifier) (qualified_identifier)
    (template_type) (struct_specifier name: (_)) (union_specifier name: (_))
    (enum_specifier name: (_)) (class_specifier name: (_))
  ]
  declarator: (type_identifier) @name @definition.type)
(type_definition
  declarator: [
    (pointer_declarator declarator: (type_identifier) @name @definition.type)
    (function_declarator declarator: (parenthesized_declarator
      (pointer_declarator declarator: (type_identifier) @name @definition.type)))
  ])

; Fields (data members).
(field_declaration_list
  (field_declaration declarator: (field_identifier) @name @definition.field))
(field_declaration_list
  (field_declaration declarator: (pointer_declarator declarator: (field_identifier) @name @definition.field)))
(field_declaration_list
  (field_declaration declarator: (pointer_declarator declarator: (pointer_declarator declarator: (field_identifier) @name @definition.field))))
(field_declaration_list
  (field_declaration declarator: (reference_declarator (field_identifier) @name @definition.field)))
(field_declaration_list
  (field_declaration declarator: (array_declarator declarator: (field_identifier) @name @definition.field)))
(field_declaration_list
  (field_declaration declarator: (function_declarator
    declarator: (parenthesized_declarator (pointer_declarator declarator: (field_identifier) @name @definition.field)))))

; Macros.
(preproc_def name: (identifier) @name value: (preproc_arg)) @definition.constant
(preproc_function_def name: (identifier) @name) @definition.macro

(translation_unit (declaration declarator: (identifier) @name @definition.variable))
(translation_unit (declaration declarator: (init_declarator declarator: (identifier) @name @definition.variable)))
(translation_unit (declaration declarator: (pointer_declarator declarator: (identifier) @name @definition.variable)))
(translation_unit (declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @name @definition.variable))))
(translation_unit (declaration declarator: (reference_declarator (identifier) @name @definition.variable)))
(translation_unit (declaration declarator: (init_declarator declarator: (reference_declarator (identifier) @name @definition.variable))))
(translation_unit (declaration declarator: (array_declarator declarator: (identifier) @name @definition.variable)))
(translation_unit (declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @name @definition.variable))))
(declaration_list (declaration declarator: (identifier) @name @definition.variable))
(declaration_list (declaration declarator: (init_declarator declarator: (identifier) @name @definition.variable)))
(declaration_list (declaration declarator: (pointer_declarator declarator: (identifier) @name @definition.variable)))
(declaration_list (declaration declarator: (init_declarator declarator: (pointer_declarator declarator: (identifier) @name @definition.variable))))
(declaration_list (declaration declarator: (reference_declarator (identifier) @name @definition.variable)))
(declaration_list (declaration declarator: (init_declarator declarator: (reference_declarator (identifier) @name @definition.variable))))
(declaration_list (declaration declarator: (array_declarator declarator: (identifier) @name @definition.variable)))
(declaration_list (declaration declarator: (init_declarator declarator: (array_declarator declarator: (identifier) @name @definition.variable))))
