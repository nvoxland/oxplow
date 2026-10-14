; What a Rust file defines (`.context/language-support.md` "Symbols").
; Seeded from tree-sitter-rust's tags.scm; adds fields, consts, statics,
; enum variants and associated types, and names `impl` blocks as
; containers.

(struct_item name: (type_identifier) @name) @definition.struct
(union_item name: (type_identifier) @name) @definition.struct
(enum_item name: (type_identifier) @name) @definition.enum
(enum_variant name: (identifier) @name) @definition.enum_member
(trait_item name: (type_identifier) @name) @definition.trait
(type_item name: (type_identifier) @name) @definition.type
(associated_type name: (type_identifier) @name) @definition.type
(mod_item name: (identifier) @name) @definition.module
(macro_definition name: (identifier) @name) @definition.macro
(function_item name: (identifier) @name) @definition.function
(function_signature_item name: (identifier) @name) @definition.function
(const_item name: (identifier) @name) @definition.constant
(static_item name: (identifier) @name) @definition.constant
(field_declaration name: (field_identifier) @name) @definition.field

(impl_item trait: (_) @container.trait type: (_) @container.name) @container
(impl_item !trait type: (_) @container.name) @container
