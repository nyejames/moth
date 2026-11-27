//! Canonical builtin `Error` type manifest and helpers.
//!
//! WHAT: owns the language-level builtin error declarations, reserved symbols, field names,
//! and lookup helpers used across AST/HIR/backend lowering.
//! WHY: builtin error metadata should be centralized so parser/lowering/backend code cannot drift
//! on the public error type shape.

use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, DeclarationShell, DeclarationValue, NodeKind,
};
use crate::compiler_frontend::ast::{ScopeContext, ScopeDeclarationRef};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
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

struct ErrorField {
    name: &'static str,
    ty: ErrorFieldType,
}

enum ErrorFieldType {
    String,
    Unsigned { scalar: FixedScalar, default: u32 },
}

impl ErrorFieldType {
    fn type_id(&self) -> TypeId {
        match *self {
            Self::String => builtin_type_ids::STRING,
            Self::Unsigned { scalar, .. } => builtin_type_ids::fixed_scalar(scalar),
        }
    }

    fn canonical_type(&self) -> CanonicalTypeIdentity {
        CanonicalTypeIdentity::Builtin(match *self {
            Self::String => CanonicalBuiltinType::String,
            Self::Unsigned { scalar, .. } => CanonicalBuiltinType::FixedScalar(scalar),
        })
    }
}

/// Declaration order also defines the ordinals of shared call-summary field selectors.
static ERROR_FIELDS: [ErrorField; 2] = [
    ErrorField {
        name: ERROR_FIELD_MESSAGE,
        ty: ErrorFieldType::String,
    },
    ErrorField {
        name: ERROR_FIELD_CODE,
        ty: ErrorFieldType::Unsigned {
            scalar: FixedScalar::U32,
            default: 0,
        },
    },
];

pub(crate) fn builtin_error_field_type(ordinal: u32) -> Option<CanonicalTypeIdentity> {
    Some(ERROR_FIELDS.get(ordinal as usize)?.ty.canonical_type())
}

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

    let mut folded_defaults_by_field_path = FxHashMap::default();
    let mut error_fields = Vec::with_capacity(ERROR_FIELDS.len());
    for field in &ERROR_FIELDS {
        let path = path_fork
            .try_intern_child(error_path, string_table.intern(field.name))
            .expect("builtin Error field path must fit the path table");
        error_fields.push(required_field(path, field.ty.type_id(), None));
        if let ErrorFieldType::Unsigned { scalar, default } = field.ty {
            folded_defaults_by_field_path.insert(
                path,
                PublicFoldedValue::FixedScalar(
                    FixedScalarValue::unsigned(scalar, u64::from(default))
                        .expect("builtin Error field default must fit its fixed-scalar domain"),
                ),
            );
        }
    }

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
) -> Result<ResolvedBuiltinType, InfrastructureFailure> {
    resolve_builtin_named_type(context, ERROR_TYPE_NAME, span, string_table)
}

fn resolve_builtin_named_type(
    context: &ScopeContext,
    type_name: &str,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> Result<ResolvedBuiltinType, InfrastructureFailure> {
    let symbol = string_table.intern(type_name);
    let Some(reference) = context.get_reference(&symbol) else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("Builtin type '{type_name}' is missing from this compilation context."),
        ));
    };
    let type_id = match reference {
        ScopeDeclarationRef::Local { declaration, .. } => declaration.type_id,
        ScopeDeclarationRef::Shared(declaration) => declaration.type_id,
        ScopeDeclarationRef::PendingNumeric { .. } => {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Builtin type '{type_name}' resolved to a pending numeric binding at {span:?}."
                ),
            ));
        }
        ScopeDeclarationRef::RouteParameter(_) => {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("Builtin type '{type_name}' resolved to a Route parameter at {span:?}."),
            ));
        }
    };

    // The manifest registers the builtin error declaration before its nominal type identity
    // exists, so a placeholder id here means this symbol resolved before registration. The
    // display spelling cannot express that: a struct shell still spells `runtime_struct(NONE)`.
    if type_id == builtin_type_ids::NONE {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "Builtin type '{type_name}' resolved to a declaration without a canonical type identity at {span:?}.",
            ),
        ));
    }

    Ok(ResolvedBuiltinType { type_id })
}

fn type_declaration(id: PathId, type_id: TypeId, span: Option<SourceSpan>) -> Declaration {
    Declaration {
        id,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableReference,
        value: DeclarationValue::Shell(DeclarationShell::Struct),
        binding_span: span,
        config_contract: None,
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
        config_contract: None,
    }
}

#[cfg(test)]
#[path = "tests/error_type_tests.rs"]
mod error_type_tests;
