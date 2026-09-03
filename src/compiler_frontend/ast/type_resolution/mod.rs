//! AST type-resolution helpers for signatures and struct fields.
//!
//! WHAT: resolves AST `NamedType` placeholders to concrete declaration-backed `DataType`s,
//!       then lowers those `DataType`s into canonical `TypeId`s in the active `TypeEnvironment`.
//! WHY: AST emission and receiver-method validation require fully resolved types up front.
//!
//! ## DataType boundary note
//!
//! This module works with `DataType` as the parse-level type representation. All `DataType`
//! matches here are part of the parse-to-semantic resolution boundary: they inspect parse
//! syntax (type parameters, generic instances, nominal paths) in order to produce canonical
//! `TypeId`s. No `DataType` match in this file should be used for executable semantic decisions
//! after resolution is complete.

use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::{DataType, ReceiverKey};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

pub(crate) type TypeResolutionResult<T> = Result<T, Box<CompilerDiagnostic>>;

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
    collect_type_parameter_ids_from_type, validate_generic_parameters_used,
};
pub(crate) use recursive_types::{
    validate_no_recursive_generic_type, validate_no_recursive_runtime_structs,
};
// Re-export the context/input/result types so callers outside this module can keep
// importing them from `ast::type_resolution` without knowing which submodule owns them.
pub(crate) use collections::fold_collection_capacity;
pub(crate) use context::{
    ResolvedTypeAlias, ResolvedTypeAnnotation, TypeResolutionContext, TypeResolutionContextInputs,
};
pub(crate) use maps::validate_map_key_type;
pub(crate) use resolve_type::resolve_diagnostic_type_to_type_id;
#[cfg(test)]
pub(crate) use resolve_type::resolve_diagnostic_type_to_type_id_opt;
pub(crate) use resolve_type::{
    resolve_diagnostic_type_to_type_id_checked, resolve_parsed_type_annotation, resolve_type,
};
pub(crate) use signatures::resolve_function_signature;
pub(crate) use struct_fields::{
    StructFieldResolutionError, resolve_struct_constructor_shell_types, resolve_struct_field_types,
};

/// Resolve a declaration type with the shared type-resolution context.
pub(crate) fn resolve_named_signature_type(
    data_type: &DataType,
    location: &SourceLocation,
    type_resolution_context: &mut TypeResolutionContext<'_>,
    string_table: &StringTable,
) -> TypeResolutionResult<DataType> {
    resolve_type(data_type, location, type_resolution_context, string_table)
}
