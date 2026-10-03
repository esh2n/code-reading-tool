; A function inside `impl Type` or `impl<T> Type<T>` belongs to Type.
(impl_item
  type: [(type_identifier) @owner
         (generic_type type: (type_identifier) @owner)]
  body: (declaration_list (function_item) @method))
