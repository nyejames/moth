//! AST type-resolution helpers for retained syntax and canonical type identity.
//!
//! WHAT: resolves authored `ParsedTypeRef` values directly to canonical `TypeId`s in the active
//!       `TypeEnvironment`, while retaining source syntax in its owning declaration/signature.
//! WHY: AST emission and receiver-method validation share canonical semantic IDs interned by the
//!      active `TypeEnvironment`.

use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::datatypes::ReceiverKey;

pub(crate) type TypeResolutionResult<T> = Result<T, DiagnosticDraft>;

/// Function signature after resolving all named types and receiver metadata.
#[derive(Clone)]
pub(crate) struct ResolvedFunctionSignature {
    pub(crate) receiver: Option<ReceiverKey>,
    pub(crate) signature: FunctionSignature,
}

mod aliases;
mod choice_variants;
mod collections;
mod context;
mod generic_parameters;
mod generics;
mod lookup;
mod maps;
mod recursive_types;
mod resolve_type;
mod signatures;
mod struct_fields;

pub(crate) use choice_variants::resolve_choice_variant_payload_types;
pub(crate) use generic_parameters::{
    GenericParameterScopeBuildInput, build_generic_parameter_scope,
    collect_type_parameter_ids_from_choice_variants, collect_type_parameter_ids_from_declarations,
    validate_generic_parameters_used,
};
pub(crate) use recursive_types::{
    validate_no_recursive_generic_type, validate_no_recursive_runtime_structs,
};
// Re-export the context/input/result types so callers outside this module can keep
// importing them from `ast::type_resolution` without knowing which submodule owns them.
pub(crate) use collections::fold_collection_capacity;
pub(crate) use context::{
    ResolvedTypeAlias, ResolvedTypeAnnotation, TypeResolutionContext, TypeResolutionContextInputs,
    TypeResolutionFailureKind,
};
pub(crate) use maps::validate_map_key_type;
pub(crate) use resolve_type::{
    resolve_parsed_type_annotation, resolve_parsed_type_annotation_with_failure_kind,
    reusable_type_id_for_parsed_ref,
};
pub(crate) use signatures::{intern_resolved_function_signature, resolve_function_signature};
pub(crate) use struct_fields::{
    StructFieldResolutionError, resolve_struct_constructor_shell_types, resolve_struct_field_types,
};
