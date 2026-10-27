//! Generic-parameter validation helpers for AST type resolution.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::TypeResolutionResult;
use crate::compiler_frontend::compiler_messages::InvalidDeclarationReason;
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameterList, GenericParameterScope, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId, builtin_type_ids};
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::binding_environment::SourceDeclarationTarget;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};

// ---------------------------
//  Type Parameter Collection
// ---------------------------

pub(crate) fn collect_type_parameter_ids_from_type(
    type_id: TypeId,
    canonical_by_local: &FxHashMap<TypeParameterId, GenericParameterId>,
    type_environment: &TypeEnvironment,
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    if type_id == builtin_type_ids::NONE {
        return;
    }

    match type_environment.get(type_id) {
        Some(TypeDefinition::GenericParameter(parameter)) => {
            if let Some((local_id, _)) = canonical_by_local
                .iter()
                .find(|(_, canonical_id)| **canonical_id == parameter.id)
            {
                used_parameters.insert(*local_id);
            }
        }

        Some(TypeDefinition::Constructed(constructed)) => {
            for argument in &constructed.arguments {
                collect_type_parameter_ids_from_type(
                    *argument,
                    canonical_by_local,
                    type_environment,
                    used_parameters,
                );
            }
        }

        Some(TypeDefinition::GenericInstance(instance)) => {
            for argument in &instance.arguments {
                collect_type_parameter_ids_from_type(
                    *argument,
                    canonical_by_local,
                    type_environment,
                    used_parameters,
                );
            }
        }

        Some(
            TypeDefinition::Builtin(_)
            | TypeDefinition::Struct(_)
            | TypeDefinition::Choice(_)
            | TypeDefinition::Function(_)
            | TypeDefinition::External(_)
            | TypeDefinition::AnonymousConstRecordMarker,
        )
        | None => {}
    }
}

pub(crate) fn collect_type_parameter_ids_from_declarations(
    declarations: &[Declaration],
    canonical_by_local: &FxHashMap<TypeParameterId, GenericParameterId>,
    type_environment: &TypeEnvironment,
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    for declaration in declarations {
        collect_type_parameter_ids_from_type(
            declaration.type_id,
            canonical_by_local,
            type_environment,
            used_parameters,
        );
    }
}

pub(crate) fn collect_type_parameter_ids_from_choice_variants(
    variants: &[ChoiceVariant],
    canonical_by_local: &FxHashMap<TypeParameterId, GenericParameterId>,
    type_environment: &TypeEnvironment,
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    for variant in variants {
        if let ChoiceVariantPayload::Record { fields } = &variant.payload {
            collect_type_parameter_ids_from_declarations(
                fields,
                canonical_by_local,
                type_environment,
                used_parameters,
            );
        }
    }
}

// ----------------------------
//  Generic Parameter Handling
// ----------------------------

pub(crate) struct GenericParameterScopeBuildInput<'a> {
    pub(crate) generic_parameters: &'a GenericParameterList,
    pub(crate) canonical_by_local: Option<&'a FxHashMap<TypeParameterId, GenericParameterId>>,
    pub(crate) source_id: Option<SourceId>,
    pub(crate) visible_source_bindings: &'a FxHashMap<StringId, SourceDeclarationTarget>,
    pub(crate) visible_type_aliases: &'a FxHashMap<StringId, SourceDeclarationTarget>,
    pub(crate) visible_external_symbols: &'a FxHashMap<StringId, ExternalSymbolId>,
    pub(crate) declaration_table: &'a TopLevelDeclarationTable,
    pub(crate) generic_declarations_by_path: &'a FxHashMap<PathId, GenericDeclarationKind>,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) string_table: &'a StringTable,
}

pub(crate) fn build_generic_parameter_scope(
    input: GenericParameterScopeBuildInput<'_>,
) -> TypeResolutionResult<Option<GenericParameterScope>> {
    let GenericParameterScopeBuildInput {
        generic_parameters,
        canonical_by_local,
        source_id: _source_id,
        visible_source_bindings,
        visible_type_aliases,
        visible_external_symbols,
        declaration_table,
        generic_declarations_by_path,
        type_environment,
        string_table,
    } = input;

    if generic_parameters.is_empty() {
        return Ok(None);
    }

    let mut forbidden_names = FxHashSet::default();
    forbidden_names.extend(visible_type_aliases.keys().copied());

    for (name, symbol_id) in visible_external_symbols {
        if matches!(symbol_id, ExternalSymbolId::Type(_)) {
            forbidden_names.insert(*name);
        }
    }

    for (name, path) in visible_source_bindings {
        if path_is_visible_type(
            path,
            declaration_table,
            generic_declarations_by_path,
            type_environment,
        ) {
            forbidden_names.insert(*name);
        }
    }

    GenericParameterScope::from_parameter_list(
        generic_parameters,
        canonical_by_local,
        &forbidden_names,
        string_table,
    )
    .map_err(|failure| {
        let span = failure.parameter_span.or_else(|| {
            let parameter_name = match &failure.reason {
                InvalidDeclarationReason::InvalidGenericParameterName { parameter_name }
                | InvalidDeclarationReason::GenericParameterNameCollision { parameter_name }
                | InvalidDeclarationReason::ReservedGenericParameterName { parameter_name }
                | InvalidDeclarationReason::DuplicateGenericParameter { parameter_name } => {
                    *parameter_name
                }
                _ => return None,
            };
            generic_parameters
                .parameters
                .iter()
                .find(|parameter| parameter.name == parameter_name)
                .and_then(|parameter| parameter.span)
        });
        names_declarations::invalid_declaration(failure.reason, None, span)
    })
    .map(Some)
}

fn path_is_visible_type(
    path: &PathId,
    declaration_table: &TopLevelDeclarationTable,
    generic_declarations_by_path: &FxHashMap<PathId, GenericDeclarationKind>,
    type_environment: &TypeEnvironment,
) -> bool {
    if let Some(kind) = generic_declarations_by_path.get(path) {
        return matches!(
            kind,
            GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
        );
    }

    // Nominal identities are registered before any generic-parameter scope is built, so the
    // declaration's canonical type answers this question without reading a display spelling.
    declaration_table
        .get_by_path(path)
        .is_some_and(|declaration| {
            matches!(
                type_environment.get(declaration.type_id),
                Some(TypeDefinition::Struct(_) | TypeDefinition::Choice(_))
            )
        })
}

pub(crate) fn validate_generic_parameters_used(
    generic_parameters: &GenericParameterList,
    used_parameters: &FxHashSet<TypeParameterId>,
    _declaration_path: &PathId,
    span: Option<SourceSpan>,
) -> TypeResolutionResult<()> {
    for parameter in &generic_parameters.parameters {
        if !used_parameters.contains(&parameter.id) {
            return Err(names_declarations::invalid_declaration(
                InvalidDeclarationReason::UnusedGenericParameter {
                    parameter_name: parameter.name,
                },
                None,
                span,
            ));
        }
    }

    Ok(())
}
