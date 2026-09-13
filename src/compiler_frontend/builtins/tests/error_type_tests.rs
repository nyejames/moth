//! Builtin error type manifest unit tests.
//!
//! WHAT: validates canonical error type registration and reserved symbol enforcement.
//! WHY: builtin error metadata must stay consistent across parser/lowering/backend code.

use crate::compiler_frontend::ast::expressions::expression::ExpressionKind;
use crate::compiler_frontend::builtins::error_type::{
    ERROR_FIELD_CODE, ERROR_FIELD_MESSAGE, ERROR_TYPE_NAME, is_reserved_builtin_symbol,
    register_builtin_error_types,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn registers_builtin_error_manifest_with_canonical_symbols() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let manifest = register_builtin_error_types(&mut path_fork, &mut string_table);
    assert_eq!(manifest.declarations.len(), 1);
    assert_eq!(manifest.visible_symbol_paths.len(), 1);

    let error_path =
        crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
            &mut path_fork,
            &mut string_table,
        );
    let error_fields = manifest
        .resolved_struct_fields_by_path
        .get(&error_path)
        .expect("Error fields should be registered");
    let mut field_names = error_fields
        .iter()
        .map(|field| {
            path_fork
                .component(field.id)
                .map(|id| string_table.resolve(id).to_owned())
                .expect("field names should exist")
        })
        .collect::<Vec<_>>();
    field_names.sort();
    assert_eq!(
        field_names,
        vec![ERROR_FIELD_CODE.to_owned(), ERROR_FIELD_MESSAGE.to_owned()]
    );
    let code_field = error_fields
        .iter()
        .find(|field| {
            path_fork
                .component(field.id)
                .map(|id| string_table.resolve(id))
                == Some(ERROR_FIELD_CODE)
        })
        .expect("Error.code should be registered");
    assert!(matches!(code_field.value.kind, ExpressionKind::Int(0)));
}

#[test]
fn reserves_builtin_error_symbol_names() {
    assert!(is_reserved_builtin_symbol(ERROR_TYPE_NAME));
    assert!(!is_reserved_builtin_symbol("ErrorKind"));
    assert!(!is_reserved_builtin_symbol("ErrorLocation"));
    assert!(!is_reserved_builtin_symbol("StackFrame"));
    assert!(!is_reserved_builtin_symbol("UserError"));
}
