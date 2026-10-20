//! Recursive generic and runtime struct validation for AST type resolution.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::TypeResolutionResult;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use rustc_hash::{FxHashMap, FxHashSet};

pub(crate) fn validate_no_recursive_generic_type(
    declaration_path: &PathId,
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    span: Option<SourceSpan>,
) -> TypeResolutionResult<()> {
    if !type_id_references_nominal_path(type_id, declaration_path, type_environment) {
        return Ok(());
    }

    Err(CompilerDiagnostic::invalid_declaration(
        InvalidDeclarationReason::RecursiveGenericType,
        None,
        span,
    ))
}

fn type_id_references_nominal_path(
    type_id: TypeId,
    declaration_path: &PathId,
    type_environment: &TypeEnvironment,
) -> bool {
    match type_environment.get(type_id) {
        Some(TypeDefinition::GenericInstance(instance)) => {
            type_environment
                .nominal_path_by_id(instance.base)
                .is_some_and(|path| path == declaration_path)
                || instance.arguments.iter().any(|argument| {
                    type_id_references_nominal_path(*argument, declaration_path, type_environment)
                })
        }

        Some(TypeDefinition::Constructed(constructed)) => {
            constructed.arguments.iter().any(|argument| {
                type_id_references_nominal_path(*argument, declaration_path, type_environment)
            })
        }

        Some(TypeDefinition::Struct(definition)) => definition.path == *declaration_path,
        Some(TypeDefinition::Choice(definition)) => definition.path == *declaration_path,

        Some(
            TypeDefinition::Builtin(_)
            | TypeDefinition::Function(_)
            | TypeDefinition::External(_)
            | TypeDefinition::GenericParameter(_)
            | TypeDefinition::AnonymousConstRecordMarker,
        )
        | None => false,
    }
}

// --------------------------------
//  Runtime struct cycle validation
// --------------------------------

fn collect_runtime_struct_dependencies(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
    dependencies: &mut FxHashSet<PathId>,
) {
    // Runtime-cycle validation follows nominal structs and their inline type arguments.
    match type_environment.get(type_id) {
        Some(TypeDefinition::Struct(definition)) if !definition.const_record => {
            dependencies.insert(definition.path);
        }

        Some(TypeDefinition::Constructed(constructed)) => {
            for argument in &constructed.arguments {
                collect_runtime_struct_dependencies(*argument, type_environment, dependencies);
            }
        }

        Some(TypeDefinition::GenericInstance(instance)) => {
            if let Some(TypeDefinition::Struct(definition)) = type_environment
                .type_id_for_nominal_id(instance.base)
                .and_then(|base_type_id| type_environment.get(base_type_id))
                && !definition.const_record
            {
                dependencies.insert(definition.path);
            }

            for argument in &instance.arguments {
                collect_runtime_struct_dependencies(*argument, type_environment, dependencies);
            }
        }

        Some(
            TypeDefinition::Builtin(_)
            | TypeDefinition::Struct(_)
            | TypeDefinition::Choice(_)
            | TypeDefinition::Function(_)
            | TypeDefinition::External(_)
            | TypeDefinition::GenericParameter(_)
            | TypeDefinition::AnonymousConstRecordMarker,
        )
        | None => {}
    }
}

/// Reject runtime struct cycles that would make concrete layout impossible to lower.
pub(crate) fn validate_no_recursive_runtime_structs(
    struct_fields_by_path: &FxHashMap<PathId, Vec<Declaration>>,
    type_environment: &TypeEnvironment,
    path_fork: &PathInternerFork,
) -> TypeResolutionResult<()> {
    // WHY: V1 runtime structs do not support recursive layout semantics yet.
    // These cycles must fail in AST construction with a targeted rule error.
    fn visit(
        current: &PathId,
        struct_fields_by_path: &FxHashMap<PathId, Vec<Declaration>>,
        type_environment: &TypeEnvironment,
        path_fork: &PathInternerFork,
        visiting: &mut Vec<PathId>,
        visited: &mut FxHashSet<PathId>,
    ) -> TypeResolutionResult<()> {
        if visited.contains(current) {
            return Ok(());
        }

        if let Some(index) = visiting.iter().position(|path| path == current) {
            let cycle = visiting[index..]
                .iter()
                .map(|path| {
                    path_fork
                        .component(*path)
                        .expect("runtime struct cycle paths must have final components")
                })
                .collect::<Vec<_>>();
            let cycle_span = struct_fields_by_path
                .get(current)
                .and_then(|fields| fields.first())
                .and_then(|field| field.binding_span);

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
                    field.type_id,
                    type_environment,
                    &mut dependencies,
                );

                for dependency in dependencies {
                    if struct_fields_by_path.contains_key(&dependency) {
                        visit(
                            &dependency,
                            struct_fields_by_path,
                            type_environment,
                            path_fork,
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
            type_environment,
            path_fork,
            &mut visiting,
            &mut visited,
        )?;
    }

    Ok(())
}
