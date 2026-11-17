; Outline for Java. Based on tree-sitter-java's bundled tags.scm, extended with enums,
; records, constructors, fields and enum constants. Capture conventions are documented in
; src/lang/generic.rs.

((class_declaration
  name: (identifier) @name) @definition.class
 (#set! exported "modifier:public"))

((record_declaration
  name: (identifier) @name) @definition.class
 (#set! exported "modifier:public"))

((interface_declaration
  name: (identifier) @name) @definition.interface
 (#set! exported "modifier:public"))

((annotation_type_declaration
  name: (identifier) @name) @definition.interface
 (#set! exported "modifier:public"))

((enum_declaration
  name: (identifier) @name) @definition.enum
 (#set! exported "modifier:public"))

(enum_constant
  name: (identifier) @name) @definition.constant

((method_declaration
  name: (identifier) @name) @definition.method
 (#set! exported "modifier:public"))

((constructor_declaration
  name: (identifier) @name) @definition.constructor
 (#set! exported "modifier:public"))

((field_declaration
  declarator: (variable_declarator
    name: (identifier) @name)) @definition.field
 (#set! exported "modifier:public"))

(method_invocation
  object: (_)? @receiver
  name: (identifier) @name
  arguments: (argument_list)) @reference.call

(object_creation_expression
  type: [
    (type_identifier) @name
    (generic_type (type_identifier) @name)
    (scoped_type_identifier (type_identifier) @name .)
  ]) @reference.call
