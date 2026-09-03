//! Canonical builtin `Error` type manifest and helpers.
//!
//! WHAT: owns the language-level builtin error declarations, reserved symbols, field names,
//! and lookup helpers used across AST/HIR/backend lowering.
//! WHY: builtin error metadata should be centralized so parser/lowering/backend code cannot drift
//! on the public error type shape.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, type_id_hint_for_diagnostic_type,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};

pub(crate) const ERROR_TYPE_NAME: &str = "Error";

pub(crate) const ERROR_FIELD_MESSAGE: &str = "message";
pub(crate) const ERROR_FIELD_CODE: &str = "code";

/// Builtin type lookup result carrying its canonical semantic identity.
///
/// WHAT: builtin parsers need the canonical `TypeId` for call validation and constructed result
///      types.
/// WHY: keeping the semantic type at the lookup boundary avoids later reverse bridges from
///      display-only representations back into type identity.
#[derive(Clone)]
pub(crate) struct ResolvedBuiltinType {
    pub(crate) type_id: TypeId,
}

/// Canonical builtin error declarations and visibility metadata.
///
/// WHAT: bundles every AST-time registration artifact required for builtin error type support.
/// WHY: AST orchestration should consume one manifest instead of manually reconstructing builtin
/// declarations and field maps.
pub(crate) struct BuiltinErrorManifest {
    pub(crate) visible_symbol_paths: FxHashSet<InternedPath>,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<InternedPath, Vec<Declaration>>,
    pub(crate) struct_source_by_path: FxHashMap<InternedPath, InternedPath>,
    pub(crate) ast_struct_nodes: Vec<AstNode>,
}

pub(crate) fn is_reserved_builtin_symbol(name: &str) -> bool {
    matches!(name, ERROR_TYPE_NAME)
}

pub(crate) fn builtin_error_type_path(string_table: &mut StringTable) -> InternedPath {
    InternedPath::from_single_str(ERROR_TYPE_NAME, string_table)
}

pub(crate) fn register_builtin_error_types(string_table: &mut StringTable) -> BuiltinErrorManifest {
    let location = SourceLocation::default();

    let error_path = builtin_error_type_path(string_table);

    let mut visible_symbol_paths = FxHashSet::default();
    visible_symbol_paths.insert(error_path.to_owned());

    let error_fields = vec![
        required_field(
            error_path.join_str(ERROR_FIELD_MESSAGE, string_table),
            DataType::StringSlice,
            location.clone(),
        ),
        defaulted_int_field(
            error_path.join_str(ERROR_FIELD_CODE, string_table),
            0,
            location.clone(),
        ),
    ];

    let declarations = vec![type_declaration(
        error_path.to_owned(),
        DataType::runtime_struct(error_path.to_owned(), builtin_type_ids::NONE),
        location.clone(),
    )];

    let mut resolved_struct_fields_by_path = FxHashMap::default();
    resolved_struct_fields_by_path.insert(error_path.to_owned(), error_fields);

    let mut struct_source_by_path = FxHashMap::default();
    struct_source_by_path.insert(error_path.to_owned(), InternedPath::new());

    let ast_struct_nodes = vec![AstNode {
        kind: NodeKind::StructDefinition(
            error_path.to_owned(),
            resolved_struct_fields_by_path
                .get(&error_path)
                .cloned()
                .unwrap_or_default(),
        ),
        location,
        scope: error_path.to_owned(),
    }];

    BuiltinErrorManifest {
        visible_symbol_paths,
        declarations,
        resolved_struct_fields_by_path,
        struct_source_by_path,
        ast_struct_nodes,
    }
}

pub(crate) fn resolve_builtin_error_type_typed(
    context: &ScopeContext,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<ResolvedBuiltinType, CompilerError> {
    resolve_builtin_named_type(context, ERROR_TYPE_NAME, location, string_table)
}

fn resolve_builtin_named_type(
    context: &ScopeContext,
    type_name: &str,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<ResolvedBuiltinType, CompilerError> {
    let symbol = string_table.intern(type_name);
    let Some(declaration) = context.get_reference(&symbol) else {
        return Err(CompilerError::compiler_error(format!(
            "Builtin type '{type_name}' is missing from this compilation context."
        )));
    };

    if declaration.value.diagnostic_type == DataType::Inferred {
        return Err(CompilerError::compiler_error(format!(
            "Builtin type '{type_name}' resolved to an inferred placeholder at {location:?}.",
        )));
    }

    Ok(ResolvedBuiltinType {
        type_id: declaration.value.type_id,
    })
}

fn type_declaration(
    id: InternedPath,
    data_type: DataType,
    location: SourceLocation,
) -> Declaration {
    Declaration {
        id,
        value: Expression::new(
            ExpressionKind::NoValue,
            location,
            type_id_hint_for_diagnostic_type(&data_type),
            data_type,
            ValueMode::ImmutableReference,
        ),
        config_qualifier: None,
    }
}
fn required_field(id: InternedPath, data_type: DataType, location: SourceLocation) -> Declaration {
    Declaration {
        id,
        value: Expression::no_value(location, data_type, ValueMode::ImmutableOwned),
        config_qualifier: None,
    }
}

fn defaulted_int_field(id: InternedPath, value: i32, location: SourceLocation) -> Declaration {
    Declaration {
        id,
        value: Expression::int(value, location, ValueMode::ImmutableOwned),
        config_qualifier: None,
    }
}

#[cfg(test)]
#[path = "tests/error_type_tests.rs"]
mod error_type_tests;
