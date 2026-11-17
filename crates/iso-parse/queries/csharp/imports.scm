; C# imports. `using A.B;` brings in a whole namespace (wildcard); `using X = A.B;` is an
; alias.

(using_directive
  name: (identifier) @import.alias
  "="
  [(qualified_name) (identifier) (generic_name) (alias_qualified_name)] @import.specifier)

(using_directive
  !name
  [(qualified_name) (identifier) (alias_qualified_name)] @import.specifier) @import.wildcard
