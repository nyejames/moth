//! Generic-parameter validation helpers for AST type resolution.

use crate::compiler_frontend::ast::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::type_resolution::TypeResolutionResult;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::generic_parameters::{
    GenericParameterList, GenericParameterScope, TypeParameterId,
};
use crate::compiler_frontend::datatypes::ids::GenericParameterId;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::headers::import_environment::SourceDeclarationTarget;
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, GenericDeclarationMetadata,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

// ----------------------------
//  Generic Parameter Handling
// ----------------------------

pub(crate) struct GenericParameterScopeBuildInput<'a> {
    pub(crate) generic_parameters: &'a GenericParameterList,
    pub(crate) canonical_by_local: Option<&'a FxHashMap<TypeParameterId, GenericParameterId>>,
    pub(crate) visible_source_bindings: &'a FxHashMap<StringId, SourceDeclarationTarget>,
    pub(crate) visible_type_aliases: &'a FxHashMap<StringId, SourceDeclarationTarget>,
    pub(crate) visible_external_symbols: &'a FxHashMap<StringId, ExternalSymbolId>,
    pub(crate) declaration_table: &'a TopLevelDeclarationTable,
    pub(crate) generic_declarations_by_path:
        &'a FxHashMap<InternedPath, GenericDeclarationMetadata>,
    pub(crate) string_table: &'a StringTable,
}

pub(crate) fn build_generic_parameter_scope(
    input: GenericParameterScopeBuildInput<'_>,
) -> TypeResolutionResult<Option<GenericParameterScope>> {
    let GenericParameterScopeBuildInput {
        generic_parameters,
        canonical_by_local,
        visible_source_bindings,
        visible_type_aliases,
        visible_external_symbols,
        declaration_table,
        generic_declarations_by_path,
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
        if path_is_visible_type(path, declaration_table, generic_declarations_by_path) {
            forbidden_names.insert(*name);
        }
    }

    GenericParameterScope::from_parameter_list(
        generic_parameters,
        canonical_by_local,
        &forbidden_names,
        string_table,
        "AST Construction",
    )
    .map(Some)
}

fn path_is_visible_type(
    path: &InternedPath,
    declaration_table: &TopLevelDeclarationTable,
    generic_declarations_by_path: &FxHashMap<InternedPath, GenericDeclarationMetadata>,
) -> bool {
    if let Some(metadata) = generic_declarations_by_path.get(path) {
        return matches!(
            metadata.kind,
            GenericDeclarationKind::Struct | GenericDeclarationKind::Choice
        );
    }

    declaration_table
        .get_by_path(path)
        .is_some_and(|declaration| {
            matches!(
                declaration.value.diagnostic_type,
                DataType::Struct { .. } | DataType::Choices { .. }
            )
        })
}

pub(crate) fn validate_generic_parameters_used(
    generic_parameters: &GenericParameterList,
    used_parameters: &FxHashSet<TypeParameterId>,
    declaration_path: &InternedPath,
    location: &SourceLocation,
) -> TypeResolutionResult<()> {
    for parameter in &generic_parameters.parameters {
        if !used_parameters.contains(&parameter.id) {
            return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                InvalidDeclarationReason::UnusedGenericParameter {
                    parameter_name: parameter.name,
                },
                declaration_path.name(),
                location.to_owned(),
            )));
        }
    }

    Ok(())
}

// ---------------------------
//  Type Parameter Collection
// ---------------------------

pub(crate) fn collect_type_parameter_ids_from_type(
    data_type: &DataType,
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    match data_type {
        // Direct type-parameter reference.
        DataType::TypeParameter { id, .. } => {
            used_parameters.insert(*id);
        }

        // Container types — recurse into inner types.
        DataType::GenericInstance { arguments, .. } => {
            for argument in arguments {
                collect_type_parameter_ids_from_type(argument, used_parameters);
            }
        }

        DataType::Option(inner) => collect_type_parameter_ids_from_type(inner, used_parameters),

        DataType::FallibleCarrier { success, error } => {
            collect_type_parameter_ids_from_type(success, used_parameters);
            collect_type_parameter_ids_from_type(error, used_parameters);
        }

        DataType::Returns(values) => {
            for value in values {
                collect_type_parameter_ids_from_type(value, used_parameters);
            }
        }

        // Function-like shapes — parameters and returns.
        DataType::Function(_, signature) => {
            for parameter in &signature.parameters {
                collect_type_parameter_ids_from_type(
                    &parameter.value.diagnostic_type,
                    used_parameters,
                );
            }

            for return_slot in &signature.returns {
                collect_type_parameter_ids_from_type(return_slot.data_type(), used_parameters);
            }
        }

        // Record-like shapes — fields and variants.
        DataType::Struct { .. } | DataType::Choices { .. } => {}

        DataType::Parameters(fields) => {
            collect_type_parameter_ids_from_declarations(fields, used_parameters);
        }

        _ => {}
    }
}

pub(crate) fn collect_type_parameter_ids_from_declarations(
    declarations: &[Declaration],
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    for declaration in declarations {
        collect_type_parameter_ids_from_type(&declaration.value.diagnostic_type, used_parameters);
    }
}

pub(crate) fn collect_type_parameter_ids_from_choice_variants(
    variants: &[ChoiceVariant],
    used_parameters: &mut FxHashSet<TypeParameterId>,
) {
    for variant in variants {
        if let ChoiceVariantPayload::Record { fields } = &variant.payload {
            collect_type_parameter_ids_from_declarations(fields, used_parameters);
        }
    }
}
