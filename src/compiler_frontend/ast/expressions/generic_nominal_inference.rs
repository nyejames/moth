//! Shared inference for generic struct and choice constructors.
//!
//! WHAT: maps expected types plus constructor arguments onto generic declaration parameters.
//! WHY: structs and choices use the same nominal generic rules, and both must route named
//! arguments through the shared call-slot resolver before binding type parameters.
//! Conflicting repeated-parameter bindings are propagated through the typed invalid generic
//! instantiation diagnostic, keeping structural non-matches distinct from binding conflicts.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallDiagnosticContext, CallValidationError, expectations_from_constructor_fields,
    resolve_call_argument_slots_typed,
};
use crate::compiler_frontend::ast::expressions::constructor_views::ConstructorField;
use crate::compiler_frontend::ast::generic_bounds::{
    GenericBoundEvidenceContext, validate_nominal_generic_bound_evidence,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, GenericInferenceSubject, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::{
    GenericParameter as EnvironmentGenericParameter, TypeEnvironment,
};
use crate::compiler_frontend::datatypes::generic_bindings::{BindingConflict, GenericTypeBindings};
use crate::compiler_frontend::datatypes::generic_identity_bridge::{
    GenericInstantiationKey, TypeIdentityKey,
};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationMetadata;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::FxHashMap;

pub(crate) enum GenericNominalTemplate<'a> {
    StructFields(&'a [ConstructorField]),
    ChoiceVariants(&'a [ChoiceVariantDefinition]),
}

pub(crate) struct GenericNominalConstructorInput<'a> {
    pub nominal_path: &'a InternedPath,
    pub display_name: &'a str,
    pub metadata: &'a GenericDeclarationMetadata,
    pub template: GenericNominalTemplate<'a>,
    pub constructor_fields: Option<&'a [ConstructorField]>,
    pub raw_args: Option<&'a [CallArgument]>,
    pub diagnostics: CallDiagnosticContext<'a>,
    pub location: SourceLocation,
}

pub(crate) struct GenericNominalInference {
    /// The interned generic instance TypeId, if inference succeeded.
    pub instance_type_id: Option<TypeId>,
    /// HIR/diagnostic bridge key derived from the canonical inferred TypeId arguments.
    pub instance_key: Option<GenericInstantiationKey>,
}

pub(crate) fn infer_generic_nominal_constructor(
    input: GenericNominalConstructorInput<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<GenericNominalInference, CallValidationError> {
    let mut bindings = GenericTypeBindings::new();
    let mut evidence_locations = NominalBindingEvidenceLocations::new();

    // ------------------------
    //  Collect type bindings
    // ------------------------
    // First from the expected result type (contextual type information),
    // then from the constructor arguments themselves.
    collect_expected_type_bindings(
        &input,
        context,
        type_interner.environment(),
        &mut bindings,
        &mut evidence_locations,
        string_table,
    )?;
    collect_constructor_argument_bindings(
        &input,
        type_interner.environment(),
        &mut bindings,
        &mut evidence_locations,
        string_table,
    )?;

    // ------------------------
    //  Resolve parameters
    // ------------------------
    let canonical_parameters =
        canonical_parameters_for_nominal(input.nominal_path, type_interner.environment());

    let mut concrete_arguments = Vec::with_capacity(input.metadata.parameters.len());
    let mut missing_parameters = Vec::new();
    for (parameter_index, parameter) in input.metadata.parameters.parameters.iter().enumerate() {
        let Some(canonical_parameter) =
            canonical_parameters.and_then(|parameters| parameters.get(parameter_index))
        else {
            missing_parameters.push(parameter.name);
            continue;
        };

        if let Some(concrete) = bindings.get(canonical_parameter.id) {
            concrete_arguments.push(concrete);
        } else {
            missing_parameters.push(parameter.name);
        }
    }

    if !missing_parameters.is_empty() {
        return Err(CompilerDiagnostic::invalid_generic_instantiation(
            Some(string_table.intern(input.display_name)),
            InvalidGenericInstantiationReason::CannotInferArguments { missing_parameters },
            input.location,
        )
        .into());
    }

    // ------------------------
    //  Build instance key
    // ------------------------
    let (instance_type_id, instance_key) = {
        let nominal_id = type_interner
            .environment()
            .nominal_id_for_path(input.nominal_path);

        let argument_keys = concrete_arguments
            .iter()
            .map(|argument| {
                type_interner
                    .environment()
                    .type_id_to_type_identity_key(*argument)
            })
            .collect::<Option<Vec<TypeIdentityKey>>>();

        let instance_key = argument_keys.map(|arguments| GenericInstantiationKey {
            base_path: input.nominal_path.to_owned(),
            arguments,
        });

        let argument_ids = concrete_arguments.clone().into_boxed_slice();
        let instance_type_id = nominal_id
            .map(|nominal_id| type_interner.intern_generic_instance(nominal_id, argument_ids));

        (instance_type_id, instance_key)
    };

    if let Some(instance_type_id) = instance_type_id {
        let evidence_context = GenericBoundEvidenceContext {
            type_environment: type_interner.environment(),
            trait_environment: Some(context.trait_environment()),
            trait_evidence_environment: Some(context.trait_evidence_environment()),
            generated_evidence_pairs: Some(context.shared.generated_evidence_pairs.as_ref()),
            visible_trait_names: context
                .file_visibility
                .as_ref()
                .map(|visibility| &visibility.visible_trait_names),
            visible_source_names: context
                .file_visibility
                .as_ref()
                .map(|visibility| &visibility.visible_source_names),
            visible_type_alias_names: context
                .file_visibility
                .as_ref()
                .map(|visibility| &visibility.visible_type_alias_names),
            visible_namespace_records: context
                .file_visibility
                .as_ref()
                .map(|visibility| &visibility.visible_namespace_records),
            resolved_type_aliases: context.shared.resolved_type_aliases.as_deref(),
        };
        validate_nominal_generic_bound_evidence(
            instance_type_id,
            input.location.clone(),
            &evidence_context,
        )
        .map_err(CallValidationError::Diagnostic)?;
    }

    Ok(GenericNominalInference {
        instance_type_id,
        instance_key,
    })
}

/// Records the first source location at which each generic parameter received a binding.
///
/// WHAT: keeps a per-parameter map used for secondary diagnostic labels when a later
/// binding conflicts with an earlier one.
/// WHY: the first evidence location lets the conflict diagnostic point the user at the
/// earlier inference that fixed the parameter before the conflicting evidence arrived.
struct NominalBindingEvidenceLocations {
    locations_by_parameter: FxHashMap<GenericParameterId, SourceLocation>,
}

impl NominalBindingEvidenceLocations {
    fn new() -> Self {
        Self {
            locations_by_parameter: FxHashMap::default(),
        }
    }

    fn previous_location(&self, parameter_id: GenericParameterId) -> Option<SourceLocation> {
        self.locations_by_parameter.get(&parameter_id).cloned()
    }

    /// Records the evidence location for every parameter that is currently bound.
    ///
    /// WHAT: uses entry-or-insert so only the first location is retained for each parameter.
    /// WHY: later evidence for the same parameter must not overwrite the first evidence
    /// location, which is the one needed for the secondary conflict label.
    fn record_first_bindings(
        &mut self,
        canonical_parameters: Option<&[EnvironmentGenericParameter]>,
        bindings: &GenericTypeBindings,
        location: SourceLocation,
    ) {
        let Some(parameters) = canonical_parameters else {
            return;
        };

        for parameter in parameters {
            if bindings.get(parameter.id).is_some() {
                self.locations_by_parameter
                    .entry(parameter.id)
                    .or_insert_with(|| location.clone());
            }
        }
    }
}

/// Shared state for fallible nominal binding collection with evidence tracking.
struct NominalBindingEvidenceContext<'a> {
    nominal_path: &'a InternedPath,
    display_name: &'a str,
    bindings: &'a mut GenericTypeBindings,
    evidence_locations: &'a mut NominalBindingEvidenceLocations,
    type_environment: &'a TypeEnvironment,
    string_table: &'a mut StringTable,
}

/// Collect generic parameter bindings from every expected type in the surrounding context.
///
/// WHAT: when the compiler already knows the nominal type being constructed (e.g. from a
/// variable declaration or function return type), the concrete generic arguments in that
/// expected type can be used to infer some or all of the constructor's generic parameters.
/// WHY: this is the primary inference source; constructor-argument inference only fills
/// gaps that the expected type leaves ambiguous.
fn collect_expected_type_bindings(
    input: &GenericNominalConstructorInput<'_>,
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut NominalBindingEvidenceLocations,
    string_table: &mut StringTable,
) -> Result<(), CallValidationError> {
    let mut evidence_context = NominalBindingEvidenceContext {
        nominal_path: input.nominal_path,
        display_name: input.display_name,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
    };

    for &expected_type_id in &context.expected_result_type_ids {
        match type_environment.get(expected_type_id) {
            // A prior generic instance of the same nominal type gives us direct argument mappings.
            Some(TypeDefinition::GenericInstance(instance)) => {
                let Some(base_path) = type_environment.nominal_path_by_id(instance.base) else {
                    continue;
                };
                if base_path != input.nominal_path {
                    continue;
                }
                let Some(canonical_parameters) =
                    canonical_parameters_for_nominal(input.nominal_path, type_environment)
                else {
                    continue;
                };

                for (parameter, &argument) in
                    canonical_parameters.iter().zip(instance.arguments.iter())
                {
                    let parameter_id = parameter.id;
                    let Some(parameter_type_id) =
                        type_environment.type_id_for_generic_parameter(parameter_id)
                    else {
                        continue;
                    };
                    collect_nominal_binding_evidence(
                        &mut evidence_context,
                        parameter_type_id,
                        argument,
                        input.location.clone(),
                    )?;
                }
            }

            // A concrete struct definition of the same path lets us bind field types.
            Some(TypeDefinition::Struct(def)) if &def.path == input.nominal_path => {
                if let GenericNominalTemplate::StructFields(template_fields) = input.template {
                    let Some(expected_fields) = type_environment.fields_for(expected_type_id)
                    else {
                        continue;
                    };
                    if template_fields.len() != expected_fields.len() {
                        continue;
                    }
                    collect_pairwise_type_bindings(
                        template_fields
                            .iter()
                            .zip(expected_fields)
                            .map(|(template, expected)| (template.type_id, expected.type_id)),
                        &mut evidence_context,
                        input.location.clone(),
                    )?;
                }
            }

            // A concrete choice definition of the same path lets us bind variant payload types.
            Some(TypeDefinition::Choice(def)) if &def.path == input.nominal_path => {
                if let GenericNominalTemplate::ChoiceVariants(template_variants) = input.template {
                    let Some(expected_variants) = type_environment.variants_for(expected_type_id)
                    else {
                        continue;
                    };
                    collect_choice_variant_bindings(
                        template_variants,
                        expected_variants,
                        &mut evidence_context,
                        input.location.clone(),
                    )?;
                }
            }

            _ => {}
        }
    }

    Ok(())
}

/// Look up the canonical generic parameter list for a nominal type path.
fn canonical_parameters_for_nominal<'a>(
    nominal_path: &InternedPath,
    type_environment: &'a TypeEnvironment,
) -> Option<&'a [EnvironmentGenericParameter]> {
    let nominal_id = type_environment.nominal_id_for_path(nominal_path)?;
    let nominal_type_id = type_environment.type_id_for_nominal_id(nominal_id)?;
    let parameter_list_id = type_environment.generic_parameter_list_id_for_type(nominal_type_id)?;
    let parameter_list = type_environment.generic_parameters(parameter_list_id)?;

    Some(parameter_list.parameters.as_slice())
}

/// Collect generic parameter bindings from the constructor's actual arguments.
///
/// WHAT: after resolving named/positional call arguments against the constructor fields,
/// compare each argument's resolved type with the corresponding field's declared type
/// to discover additional generic parameter constraints.
/// WHY: this catches parameters that contextual type information alone could not infer
/// (e.g. a parameter that only appears in a field whose type is not fixed by the context).
fn collect_constructor_argument_bindings(
    input: &GenericNominalConstructorInput<'_>,
    type_environment: &TypeEnvironment,
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut NominalBindingEvidenceLocations,
    string_table: &mut StringTable,
) -> Result<(), CallValidationError> {
    let (Some(fields), Some(raw_args)) = (input.constructor_fields, input.raw_args) else {
        return Ok(());
    };

    let expectations = expectations_from_constructor_fields(fields);
    let resolved_slots = resolve_call_argument_slots_typed(
        input.diagnostics,
        raw_args,
        &expectations,
        input.location.clone(),
        string_table,
    )?;

    let mut evidence_context = NominalBindingEvidenceContext {
        nominal_path: input.nominal_path,
        display_name: input.display_name,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
    };

    for (field, slot) in fields.iter().zip(resolved_slots.iter()) {
        let Some(argument) = slot else {
            continue;
        };

        // Skip if either side lacks a resolved type_id (e.g. unresolved constant).
        if type_environment.get(field.type_id).is_none()
            || type_environment.get(argument.value.type_id).is_none()
        {
            continue;
        }

        collect_nominal_binding_evidence(
            &mut evidence_context,
            field.type_id,
            argument.value.type_id,
            argument.location.clone(),
        )?;
    }

    Ok(())
}

/// Collects one template-to-concrete binding pair and records evidence.
///
/// WHAT: unifies a template `TypeId` with a concrete `TypeId` through the fallible owner,
/// records the evidence location for newly-bound parameters on a complete structural match,
/// and converts a binding conflict into the typed conflicting-inference diagnostic.
/// WHY: structural non-matches return `Ok(false)` and stay distinct from binding conflicts,
/// which propagate as the typed invalid generic instantiation diagnostic. Evidence is
/// recorded only for a complete match so a mismatch cannot poison a later constraint.
fn collect_nominal_binding_evidence(
    context: &mut NominalBindingEvidenceContext<'_>,
    template_type_id: TypeId,
    concrete_type_id: TypeId,
    location: SourceLocation,
) -> Result<(), CallValidationError> {
    let canonical_parameters =
        canonical_parameters_for_nominal(context.nominal_path, context.type_environment);

    match context
        .type_environment
        .try_collect_type_parameter_bindings_typeid(
            template_type_id,
            concrete_type_id,
            &mut *context.bindings,
        ) {
        Ok(true) => {
            context.evidence_locations.record_first_bindings(
                canonical_parameters,
                &*context.bindings,
                location,
            );
            Ok(())
        }
        Ok(false) => {
            // Structural non-match: no new binding evidence, and the staged walk
            // left the caller's binding map unchanged. This stays a non-binding
            // mismatch, not a repeated-parameter conflict.
            Ok(())
        }
        Err(conflict) => {
            let previous_evidence_location = context
                .evidence_locations
                .previous_location(conflict.parameter_id);
            Err(nominal_binding_conflict_diagnostic(
                context.display_name,
                conflict,
                canonical_parameters,
                context.string_table,
                location,
                previous_evidence_location,
            )
            .into())
        }
    }
}

/// Pairwise generic binding collection between two `TypeId` slices.
///
/// WHAT: for each position, collects one template-to-concrete binding pair with evidence.
/// WHY: struct fields and choice payload fields both reduce to matching `TypeId` slices,
/// so one helper serves both and avoids duplicating the length guard and zipping logic.
fn collect_pairwise_type_bindings(
    type_pairs: impl IntoIterator<Item = (TypeId, TypeId)>,
    context: &mut NominalBindingEvidenceContext<'_>,
    location: SourceLocation,
) -> Result<(), CallValidationError> {
    for (template_type_id, concrete_type_id) in type_pairs {
        collect_nominal_binding_evidence(
            context,
            template_type_id,
            concrete_type_id,
            location.clone(),
        )?;
    }

    Ok(())
}

/// Pairwise generic binding collection between template choice variants and expected choice variants.
///
/// WHAT: for each variant in the choice declaration template, match it with the corresponding
/// expected variant and, if both are record payloads, delegate to `collect_pairwise_type_bindings`.
fn collect_choice_variant_bindings(
    template_variants: &[ChoiceVariantDefinition],
    expected_variants: &[ChoiceVariantDefinition],
    context: &mut NominalBindingEvidenceContext<'_>,
    location: SourceLocation,
) -> Result<(), CallValidationError> {
    if template_variants.len() != expected_variants.len() {
        return Ok(());
    }

    for (template_variant, expected_variant) in template_variants.iter().zip(expected_variants) {
        let (
            ChoiceVariantPayloadDefinition::Record {
                fields: template_fields,
            },
            ChoiceVariantPayloadDefinition::Record {
                fields: expected_fields,
            },
        ) = (&template_variant.payload, &expected_variant.payload)
        else {
            continue;
        };
        if template_fields.len() != expected_fields.len() {
            continue;
        }

        collect_pairwise_type_bindings(
            template_fields
                .iter()
                .zip(expected_fields)
                .map(|(template, expected)| (template.type_id, expected.type_id)),
            context,
            location.clone(),
        )?;
    }

    Ok(())
}

/// Builds the typed generic-inference diagnostic from a nominal binding conflict.
///
/// WHAT: resolves the parameter name from the canonical parameter list, carries the
/// conflicting `TypeId`s without rendering them, and attaches a secondary label at the
/// first evidence location when one was recorded.
/// WHY: type names are rendered later through `DiagnosticRenderContext`; the diagnostic
/// payload carries only semantic `TypeId`s and structured facts.
fn nominal_binding_conflict_diagnostic(
    display_name: &str,
    conflict: BindingConflict,
    canonical_parameters: Option<&[EnvironmentGenericParameter]>,
    string_table: &mut StringTable,
    current_evidence_location: SourceLocation,
    previous_evidence_location: Option<SourceLocation>,
) -> CompilerDiagnostic {
    let parameter_name = canonical_parameters
        .and_then(|parameters| {
            parameters
                .iter()
                .find(|parameter| parameter.id == conflict.parameter_id)
                .map(|parameter| parameter.name)
        })
        .unwrap_or_else(|| string_table.intern("<generic parameter>"));

    CompilerDiagnostic::conflicting_generic_inference(
        Some(string_table.intern(display_name)),
        GenericInferenceSubject::NominalType,
        conflict,
        parameter_name,
        current_evidence_location,
        previous_evidence_location,
    )
}
