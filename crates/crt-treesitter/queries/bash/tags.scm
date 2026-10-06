; tree-sitter-bash ships no tags query. Functions are both forms,
; `name() { … }` and `function name { … }`. A call is any command whose name
; is a plain word, builtins included as other languages count every call;
; not one built from an expansion like `$cmd`, and not the control-flow
; builtins, which other languages write as keywords rather than calls.
(function_definition name: (word) @name) @definition.function
((command name: (command_name (word) @name)) @reference.call
  (#not-any-of? @name "return" "exit" "break" "continue"))
