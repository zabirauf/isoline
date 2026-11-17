; Outline for Go. Based on tree-sitter-go's bundled tags.scm, extended with kinds the
; outline shows (struct, interface, constants) and the receiver type as @parent.
; Capture conventions are documented in src/lang/generic.rs.

((function_declaration
  name: (identifier) @name) @definition.function
 (#set! exported "capitalized"))

((method_declaration
  receiver: (parameter_list
    (parameter_declaration
      type: [
        (type_identifier) @parent
        (pointer_type (type_identifier) @parent)
        (generic_type type: (type_identifier) @parent)
        (pointer_type (generic_type type: (type_identifier) @parent))
      ]))
  name: (field_identifier) @name) @definition.method
 (#set! exported "capitalized"))

((type_spec
  name: (type_identifier) @name
  type: (struct_type)) @definition.struct
 (#set! exported "capitalized"))

((type_spec
  name: (type_identifier) @name
  type: (interface_type)) @definition.interface
 (#set! exported "capitalized"))

((type_spec
  name: (type_identifier) @name
  type: [
    (type_identifier)
    (qualified_type)
    (pointer_type)
    (map_type)
    (slice_type)
    (array_type)
    (function_type)
    (channel_type)
    (generic_type)
  ]) @definition.type
 (#set! exported "capitalized"))

((type_alias
  name: (type_identifier) @name) @definition.type
 (#set! exported "capitalized"))

((method_elem
  name: (field_identifier) @name) @definition.method
 (#set! exported "capitalized"))

(source_file
  (const_declaration
    ((const_spec
      name: (identifier) @name) @definition.constant
     (#set! exported "capitalized"))))

(source_file
  (var_declaration
    ((var_spec
      name: (identifier) @name) @definition.variable
     (#set! exported "capitalized"))))

(call_expression
  function: (identifier) @name) @reference.call

(call_expression
  function: (selector_expression
    operand: (_) @receiver
    field: (field_identifier) @name)) @reference.call
