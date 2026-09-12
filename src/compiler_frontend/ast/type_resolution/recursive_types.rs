//! Recursive generic and runtime struct validation for AST type resolution.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::TypeResolutionResult;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    GenericBaseType, GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use rustc_hash::{FxHashMap, FxHashSet};

// --------------------------
//  Type Recursion Validation
// --------------------------

pub(crate) fn validate_no_recursive_generic_type(
    declaration_path: &PathId,
    data_type: &DataType,
    span: Option<SourceSpan>,
    _string_table: &StringTable,
) -> TypeResolutionResult<()> {
    if !generic_type_references_nominal_path(data_type, declaration_path) {
        return Ok(());
    }

    return Err(CompilerDiagnostic::invalid_declaration(
        InvalidDeclarationReason::RecursiveGenericType,
        None,
        span,
    ));
}

fn generic_type_references_nominal_path(
    data_type: &DataType,
    declaration_path: &PathId,
) -> bool {
    match data_type {
        DataType::GenericInstance { base, arguments } => {
            let base_matches = matches!(
                base,
                GenericBaseType::ResolvedNominal(path) if path == declaration_path
            );

            base_matches
                || arguments.iter().any(|argument| {
                    generic_type_references_nominal_path(argument, declaration_path)
                })
        }

        DataType::Option(inner) => generic_type_references_nominal_path(inner, declaration_path),

        DataType::FallibleCarrier { success, error } => {
            generic_type_references_nominal_path(success, declaration_path)
                || generic_type_references_nominal_path(error, declaration_path)
        }

        DataType::Returns(values) => values
            .iter()
            .any(|value| generic_type_references_nominal_path(value, declaration_path)),

        DataType::Function(_, signature) => {
            signature.parameters.iter().any(|parameter| {
                generic_type_references_nominal_path(
                    &parameter.value.diagnostic_type,
                    declaration_path,
                )
            }) || signature.returns.iter().any(|return_slot| {
                generic_type_references_nominal_path(return_slot.data_type(), declaration_path)
            })
        }

        DataType::Struct {
            nominal_path,
            generic_instance_key: Some(key),
            ..
        }
        | DataType::Choices {
            nominal_path,
            generic_instance_key: Some(key),
            ..
        } => {
            nominal_path == declaration_path
                || generic_instance_key_references_nominal_path(key, declaration_path)
        }

        DataType::Struct { nominal_path, .. } | DataType::Choices { nominal_path, .. } => {
            // Callers invoke this validation only for generic declarations.
            // A direct self-nominal reference here is therefore recursive generic shape,
            // while non-generic runtime struct cycles stay on the runtime-cycle path.
            nominal_path == declaration_path
        }

        DataType::Parameters(fields) => fields.iter().any(|field| {
            generic_type_references_nominal_path(&field.value.diagnostic_type, declaration_path)
        }),

        _ => false,
    }
}

fn generic_instance_key_references_nominal_path(
    key: &GenericInstantiationKey,
    declaration_path: &PathId,
) -> bool {
    &key.base_path == declaration_path
        || key
            .arguments
            .iter()
            .any(|argument| type_identity_key_references_nominal_path(argument, declaration_path))
}

fn type_identity_key_references_nominal_path(
    key: &TypeIdentityKey,
    declaration_path: &PathId,
) -> bool {
    match key {
        TypeIdentityKey::Nominal(path) => path == declaration_path,
        TypeIdentityKey::Collection { element: inner, .. } | TypeIdentityKey::Option(inner) => {
            type_identity_key_references_nominal_path(inner, declaration_path)
        }
        TypeIdentityKey::Map { key, value } => {
            type_identity_key_references_nominal_path(key, declaration_path)
                || type_identity_key_references_nominal_path(value, declaration_path)
        }
        TypeIdentityKey::FallibleCarrier { success, error } => {
            type_identity_key_references_nominal_path(success, declaration_path)
                || type_identity_key_references_nominal_path(error, declaration_path)
        }
        TypeIdentityKey::GenericInstance(instance) => {
            generic_instance_key_references_nominal_path(instance, declaration_path)
        }
        TypeIdentityKey::Builtin(_) | TypeIdentityKey::External(_) => false,
    }
}

// --------------------------------
//  Runtime struct cycle validation
// --------------------------------

fn collect_runtime_struct_dependencies(
    data_type: &DataType,
    dependencies: &mut FxHashSet<PathId>,
) {
    // WHY: Cycle validation only cares about runtime struct-to-struct edges,
    // not scalar or constant data.
    match data_type {
        DataType::Struct {
            nominal_path,
            const_record: false,
            ..
        } => {
            dependencies.insert(nominal_path.to_owned());
        }

        DataType::Option(inner) => collect_runtime_struct_dependencies(inner, dependencies),

        DataType::FallibleCarrier { success, error } => {
            collect_runtime_struct_dependencies(success, dependencies);
            collect_runtime_struct_dependencies(error, dependencies);
        }

        DataType::GenericInstance { arguments, .. } => {
            for argument in arguments {
                collect_runtime_struct_dependencies(argument, dependencies);
            }
        }

        DataType::Returns(values) => {
            for value in values {
                collect_runtime_struct_dependencies(value, dependencies);
            }
        }

        _ => {}
    }
}

/// Reject runtime struct cycles that would make concrete layout impossible to lower.
pub(crate) fn validate_no_recursive_runtime_structs(
    struct_fields_by_path: &FxHashMap<PathId, Vec<Declaration>>,
    string_table: &StringTable,
) -> TypeResolutionResult<()> {
    // WHY: V1 runtime structs do not support recursive layout semantics yet.
    // These cycles must fail in AST construction with a targeted rule error.
    fn visit(
        current: &PathId,
        struct_fields_by_path: &FxHashMap<PathId, Vec<Declaration>>,
        string_table: &StringTable,
        visiting: &mut Vec<PathId>,
        visited: &mut FxHashSet<PathId>,
    ) -> TypeResolutionResult<()> {
        if visited.contains(current) {
            return Ok(());
        }

        if let Some(index) = visiting.iter().position(|path| path == current) {
            let cycle = visiting[index..]
                .iter()
                .map(|path| format!("{path:?}"))
                .collect::<Vec<_>>()
                .join(" -> ");
            let cycle_span = struct_fields_by_path
                .get(current)
                .and_then(|fields| fields.first())
                .and_then(|field| field.value.span);

            return Err(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::RecursiveRuntimeStruct { cycle },
                None,
                cycle_span,
            ));
        }

        visiting.push(current.to_owned());

        if let Some(fields) = struct_fields_by_path.get(current) {
            for field in fields {
                let mut dependencies = FxHashSet::default();
                collect_runtime_struct_dependencies(
                    &field.value.diagnostic_type,
                    &mut dependencies,
                );

                for dependency in dependencies {
                    if struct_fields_by_path.contains_key(&dependency) {
                        visit(
                            &dependency,
                            struct_fields_by_path,
                            string_table,
                            visiting,
                            visited,
                        )?;
                    }
                }
            }
        }

        visiting.pop();
        visited.insert(current.to_owned());
        Ok(())
    }

    let mut visited = FxHashSet::default();
    let mut visiting = Vec::new();

    for struct_path in struct_fields_by_path.keys() {
        visit(
            struct_path,
            struct_fields_by_path,
            string_table,
            &mut visiting,
            &mut visited,
        )?;
    }

    Ok(())
}
