//! Canonical builtin `Error` type manifest and helpers.
//!
//! WHAT: owns the language-level builtin error declarations, reserved symbols, field names,
//! and lookup helpers used across AST/HIR/backend lowering.
//! WHY: builtin error metadata should be centralized so parser/lowering/backend code cannot drift
//! on the public error type shape.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, DeclarationShell, DeclarationValue, NodeKind,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::ParameterContractKind;
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
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
    pub(crate) visible_symbol_paths: FxHashSet<PathId>,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) resolved_struct_fields_by_path: FxHashMap<PathId, Vec<Declaration>>,
    pub(crate) folded_defaults_by_field_path: FxHashMap<PathId, PublicFoldedValue>,
    pub(crate) struct_source_by_path: FxHashMap<PathId, PathId>,
    pub(crate) ast_struct_nodes: Vec<AstNode>,
}

pub(crate) fn is_reserved_builtin_symbol(name: &str) -> bool {
    matches!(name, ERROR_TYPE_NAME)
}

pub(crate) fn is_reserved_parameter_contract_symbol(name: &str) -> bool {
    ParameterContractKind::from_spelling(name).is_some()
}

pub(crate) fn builtin_error_type_path(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> PathId {
    path_fork
        .try_intern_portable_path(ERROR_TYPE_NAME, string_table)
        .expect("builtin Error path must fit the path table")
}

pub(crate) fn register_builtin_error_types(
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> BuiltinErrorManifest {
    let error_path = builtin_error_type_path(path_fork, string_table);

    let mut visible_symbol_paths = FxHashSet::default();
    visible_symbol_paths.insert(error_path);

    let message_path = path_fork
        .try_intern_child(error_path, string_table.intern(ERROR_FIELD_MESSAGE))
        .expect("builtin Error.message path must fit the path table");
    let code_path = path_fork
        .try_intern_child(error_path, string_table.intern(ERROR_FIELD_CODE))
        .expect("builtin Error.code path must fit the path table");

    let mut folded_defaults_by_field_path = FxHashMap::default();
    folded_defaults_by_field_path.insert(code_path, u32_field_default(0));

    let error_fields = vec![
        required_field(message_path, builtin_type_ids::STRING, None),
        defaulted_u32_field(code_path, None),
    ];

    let declarations = vec![type_declaration(error_path, builtin_type_ids::NONE, None)];

    let mut resolved_struct_fields_by_path = FxHashMap::default();
    resolved_struct_fields_by_path.insert(error_path, error_fields);

    let mut struct_source_by_path = FxHashMap::default();
    struct_source_by_path.insert(error_path, PathId::ROOT);

    let ast_struct_nodes = vec![AstNode {
        kind: NodeKind::StructDefinition(
            error_path,
            resolved_struct_fields_by_path
                .get(&error_path)
                .cloned()
                .unwrap_or_default(),
        ),
        span: None,
        scope: error_path,
    }];

    BuiltinErrorManifest {
        visible_symbol_paths,
        declarations,
        resolved_struct_fields_by_path,
        folded_defaults_by_field_path,
        struct_source_by_path,
        ast_struct_nodes,
    }
}

pub(crate) fn resolve_builtin_error_type_typed(
    context: &ScopeContext,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<ResolvedBuiltinType, CompilerError> {
    resolve_builtin_named_type(context, ERROR_TYPE_NAME, span, string_table)
}

fn resolve_builtin_named_type(
    context: &ScopeContext,
    type_name: &str,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<ResolvedBuiltinType, CompilerError> {
    let symbol = string_table.intern(type_name);
    let Some(reference) = context.get_reference(&symbol) else {
        return Err(CompilerError::compiler_error(format!(
            "Builtin type '{type_name}' is missing from this compilation context."
        )));
    };
    let Some(declaration) = reference.ready_declaration() else {
        return Err(CompilerError::compiler_error(format!(
            "Builtin type '{type_name}' resolved to a pending numeric binding at {span:?}."
        )));
    };

    // The manifest registers the builtin error declaration before its nominal type identity
    // exists, so a placeholder id here means this symbol resolved before registration. The
    // display spelling cannot express that: a struct shell still spells `runtime_struct(NONE)`.
    if declaration.type_id == builtin_type_ids::NONE {
        return Err(CompilerError::compiler_error(format!(
            "Builtin type '{type_name}' resolved to a declaration without a canonical type identity at {span:?}.",
        )));
    }

    Ok(ResolvedBuiltinType {
        type_id: declaration.type_id,
    })
}

fn type_declaration(id: PathId, type_id: TypeId, span: Option<SourceSpan>) -> Declaration {
    Declaration {
        id,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableReference,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: span,
        config_qualifier: None,
    }
}
fn required_field(id: PathId, type_id: TypeId, span: Option<SourceSpan>) -> Declaration {
    Declaration {
        id,
        type_syntax: None,
        type_id,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: span,
        config_qualifier: None,
    }
}

/// The canonical `Error.code` identity: an unsigned 32-bit fixed-scalar field. Its default
/// is carried in `folded_defaults_by_field_path` until the AST build materialises it.
fn defaulted_u32_field(id: PathId, span: Option<SourceSpan>) -> Declaration {
    Declaration {
        id,
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::U32),
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: span,
        config_qualifier: None,
    }
}

/// The owned folded default for a `U32` builtin field.
fn u32_field_default(value: u32) -> PublicFoldedValue {
    PublicFoldedValue::FixedScalar(
        FixedScalarValue::unsigned(FixedScalar::U32, u64::from(value))
            .expect("the builtin Error.code default must fit the U32 fixed-scalar domain"),
    )
}

#[cfg(test)]
#[path = "tests/error_type_tests.rs"]
mod error_type_tests;
