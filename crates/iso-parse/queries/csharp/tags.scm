; Outline for C#. Based on tree-sitter-c-sharp's bundled tags.scm, extended with structs,
; enums, records, constructors, properties, delegates and namespaces. Capture conventions
; are documented in src/lang/generic.rs.

(namespace_declaration
  name: (_) @name) @definition.module

(file_scoped_namespace_declaration
  name: (_) @name) @definition.module

((class_declaration
  name: (identifier) @name) @definition.class
 (#set! exported "modifier:public"))

((record_declaration
  name: (identifier) @name) @definition.class
 (#set! exported "modifier:public"))

((struct_declaration
  name: (identifier) @name) @definition.struct
 (#set! exported "modifier:public"))

((interface_declaration
  name: (identifier) @name) @definition.interface
 (#set! exported "modifier:public"))

((enum_declaration
  name: (identifier) @name) @definition.enum
 (#set! exported "modifier:public"))

((delegate_declaration
  name: (identifier) @name) @definition.type
 (#set! exported "modifier:public"))

((method_declaration
  name: (identifier) @name) @definition.method
 (#set! exported "modifier:public"))

((constructor_declaration
  name: (identifier) @name) @definition.constructor
 (#set! exported "modifier:public"))

((property_declaration
  name: (identifier) @name) @definition.property
 (#set! exported "modifier:public"))

(invocation_expression
  function: [
    (identifier) @name
    (generic_name (identifier) @name)
    (member_access_expression
      expression: (_) @receiver
      name: [(identifier) @name (generic_name (identifier) @name)])
  ]) @reference.call

(object_creation_expression
  type: [
    (identifier) @name
    (generic_name (identifier) @name)
    (qualified_name name: (identifier) @name)
  ]) @reference.call
