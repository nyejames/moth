//! Generic free-function call inference.
//!
//! WHAT: infers concrete type arguments for visible generic function calls, substitutes the
//! template signature, and records a request for AST emission to materialize the concrete body in
//! the consuming module.
//! WHY: generic solving belongs in AST before HIR; backends must only see ordinary concrete
//! function calls.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, ExpectedParameterType,
    ParameterExpectation, expectations_from_user_parameters, resolve_call_argument_slots_typed,
    resolve_call_arguments, resolve_call_arguments_shape_and_access,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::expressions::function_calls::parse_generic_call_arguments_typed;
use crate::compiler_frontend::ast::generic_bounds::{
    evidence_for_type, evidence_target_is_visible, generated_evidence_pair_is_selected,
    generic_parameter_declares_bound,
};
use crate::compiler_frontend::ast::generic_functions::diagnostics::{
    cannot_infer_generic_function_arguments, conflicting_generic_function_argument,
    missing_generic_function_trait_evidence, recursive_generic_function_instantiation,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericFunctionInstanceKey, GenericFunctionInstantiationRequest, GenericFunctionTemplate,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::fallible_handling::{
    FallibleCallSite, HandledFallibleCall, call_success_is_optional, non_fallible_handler_reason,
    parse_fallible_handling_suffix_for_call_expression,
};
use crate::compiler_frontend::ast::statements::functions::{FunctionSignature, ReturnSlot};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidFallibleHandlingReason,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::generic_bindings::{BindingConflict, GenericTypeBindings};
use crate::compiler_frontend::datatypes::ids::{
    GenericParameterId, GenericParameterListId, TypeId,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use rustc_hash::FxHashMap;

/// Input bundle for generic call inference.
pub(crate) struct GenericFunctionCallParseInput<'a, 'b> {
    pub(crate) token_stream: &'a mut FileTokens,
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) context: &'a ScopeContext,
    pub(crate) expected_context: GenericCallExpectedContext<'a>,
    pub(crate) value_required: bool,
    pub(crate) allow_boundary_catch: bool,
    pub(crate) call_location: SourceLocation,
    pub(crate) warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    pub(crate) type_interner: &'a mut AstTypeInterner<'b>,
    pub(crate) string_table: &'a mut StringTable,
}

/// Expected-result evidence available to generic free-function inference.
///
/// WHAT: distinguishes a direct receiving-site type from the absence of
/// expected-result evidence.
/// WHY: generic calls may infer from immediate declaration/return/then
/// boundaries, but not from later use or from an outer function parameter when
/// the generic call is nested inside another ordinary argument expression.
#[derive(Clone, Copy)]
pub(crate) enum GenericCallExpectedContext<'a> {
    ImmediateResult(&'a [TypeId]),
    None,
}

struct GenericFunctionCallFinishInput<'a, 'b> {
    token_stream: &'a mut FileTokens,
    context: &'a ScopeContext,
    call: HandledFallibleCall,
    error_return_type_id: Option<TypeId>,
    value_required: bool,
    allow_boundary_catch: bool,
    warnings: Option<&'a mut Vec<CompilerDiagnostic>>,
    type_interner: &'a mut AstTypeInterner<'b>,
    string_table: &'a mut StringTable,
}

fn parse_generic_function_call(
    input: GenericFunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallParseInput {
        token_stream,
        template,
        context,
        expected_context,
        value_required,
        allow_boundary_catch,
        call_location,
        warnings,
        type_interner,
        string_table,
    } = input;

    let parameter_expectations = expectations_from_user_parameters(&template.signature.parameters);
    let raw_arguments = parse_generic_call_arguments_typed(
        token_stream,
        context,
        type_interner,
        string_table,
        template.function_path.name(),
        &parameter_expectations,
    )?;
    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: &raw_arguments,
        expected_context,
        call_location: call_location.clone(),
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
    })?;
    let selected_evidence = validate_generic_function_bound_evidence(
        template,
        inference.key.type_arguments.as_ref(),
        context,
        type_interner.environment(),
        call_location.clone(),
    )?;

    if context.is_generic_function_instantiation_active(&inference.key) {
        return Err(recursive_generic_function_instantiation(
            template.function_path.name(),
            call_location,
        )
        .into());
    }

    let callee_name = template
        .function_path
        .name_str(string_table)
        .map(|name| name.to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations = expectations_from_user_parameters(&inference.signature.parameters);
    let type_check_context = type_interner.type_check_context();
    let arguments = resolve_call_arguments(
        CallDiagnosticContext::function(&callee_name),
        &raw_arguments,
        &expectations,
        call_location.clone(),
        CallArgumentResolutionContext {
            string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .map_err(ExpressionParseError::from)?;

    let call = HandledFallibleCall {
        name: inference.instance_path.clone(),
        args: arguments,
        result_type_ids: inference.signature.success_return_type_ids(),
        call_location: call_location.clone(),
    };

    let expression = finish_generic_function_call(GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        error_return_type_id: inference.signature.error_return_type_id(),
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    })?;

    context.record_generic_function_instantiation_request(GenericFunctionInstantiationRequest {
        declaration_identity: template.declaration_identity.clone(),
        evidence: selected_evidence,
        key: inference.key,
        instance_path: inference.instance_path.clone(),
        call_location: call_location.clone(),
    });

    Ok(expression)
}

pub(crate) fn parse_generic_function_call_expression(
    input: GenericFunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    parse_generic_function_call(input)
}

fn validate_generic_function_template_call(
    input: GenericFunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallParseInput {
        token_stream,
        template,
        context,
        expected_context,
        value_required,
        allow_boundary_catch,
        call_location,
        warnings,
        type_interner,
        string_table,
    } = input;

    let parameter_expectations = expectations_from_user_parameters(&template.signature.parameters);
    let raw_arguments = parse_generic_call_arguments_typed(
        token_stream,
        context,
        type_interner,
        string_table,
        template.function_path.name(),
        &parameter_expectations,
    )?;
    let inference = infer_generic_function_call(GenericFunctionInferenceInput {
        template,
        raw_arguments: &raw_arguments,
        expected_context,
        call_location: call_location.clone(),
        type_environment: type_interner.environment_mut_for_derived_types(),
        string_table,
    })?;
    validate_generic_function_bound_evidence(
        template,
        inference.key.type_arguments.as_ref(),
        context,
        type_interner.environment(),
        call_location.clone(),
    )?;

    let callee_name = template
        .function_path
        .name_str(string_table)
        .map(|name| name.to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations = expectations_from_user_parameters(&inference.signature.parameters);
    let arguments = resolve_call_arguments_shape_and_access(
        CallDiagnosticContext::function(&callee_name),
        &raw_arguments,
        &expectations,
        call_location.clone(),
        string_table,
        type_interner.environment(),
    )
    .map_err(ExpressionParseError::from)?;

    let call = HandledFallibleCall {
        name: template.function_path.clone(),
        args: arguments,
        result_type_ids: inference.signature.success_return_type_ids(),
        call_location: call_location.clone(),
    };

    finish_generic_function_call(GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        error_return_type_id: inference.signature.error_return_type_id(),
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    })
}

pub(crate) fn validate_generic_function_template_call_expression(
    input: GenericFunctionCallParseInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    validate_generic_function_template_call(input)
}

fn finish_generic_function_call(
    input: GenericFunctionCallFinishInput<'_, '_>,
) -> Result<Expression, ExpressionParseError> {
    let GenericFunctionCallFinishInput {
        token_stream,
        context,
        call,
        error_return_type_id,
        value_required,
        allow_boundary_catch,
        warnings,
        type_interner,
        string_table,
    } = input;

    let Some(error_return_type_id) = error_return_type_id else {
        if matches!(
            token_stream.current_token_kind(),
            TokenKind::Bang | TokenKind::Catch
        ) {
            let operand_is_optional = call_success_is_optional(
                call.result_type_ids.as_slice(),
                type_interner.environment(),
            );
            return Err(CompilerDiagnostic::invalid_fallible_handling(
                non_fallible_handler_reason(token_stream.current_token_kind(), operand_is_optional),
                token_stream.current_location(),
            )
            .into());
        }

        return Ok(call.into_plain_expression(type_interner.environment_mut_for_derived_types()));
    };

    if token_stream.current_token_kind() == &TokenKind::Bang
        || token_stream.current_token_kind() == &TokenKind::Catch
        || (matches!(token_stream.current_token_kind(), TokenKind::Symbol(_))
            && token_stream.peek_next_token() == Some(&TokenKind::Bang))
    {
        return parse_fallible_handling_suffix_for_call_expression(
            token_stream,
            context,
            FallibleCallSite {
                call,
                error_return_type_id,
                value_required,
                allow_boundary_catch,
            },
            warnings,
            type_interner,
            string_table,
        );
    }

    Err(CompilerDiagnostic::invalid_fallible_handling(
        InvalidFallibleHandlingReason::UnhandledErrorReturn,
        token_stream.current_location(),
    )
    .into())
}

pub(crate) struct GenericFunctionInferenceInput<'a> {
    pub(crate) template: &'a GenericFunctionTemplate,
    pub(crate) raw_arguments: &'a [CallArgument],
    pub(crate) expected_context: GenericCallExpectedContext<'a>,
    pub(crate) call_location: SourceLocation,
    pub(crate) type_environment: &'a mut TypeEnvironment,
    pub(crate) string_table: &'a mut StringTable,
}

pub(crate) struct GenericFunctionInference {
    pub(crate) key: GenericFunctionInstanceKey,
    pub(crate) instance_path: InternedPath,
    pub(crate) signature: FunctionSignature,
}

struct GenericBindingEvidenceLocations {
    locations_by_parameter: FxHashMap<GenericParameterId, SourceLocation>,
}

struct GenericBindingEvidenceContext<'a> {
    template: &'a GenericFunctionTemplate,
    bindings: &'a mut GenericTypeBindings,
    evidence_locations: &'a mut GenericBindingEvidenceLocations,
    type_environment: &'a TypeEnvironment,
    string_table: &'a mut StringTable,
}

impl GenericBindingEvidenceLocations {
    fn new() -> Self {
        Self {
            locations_by_parameter: FxHashMap::default(),
        }
    }

    fn previous_location(&self, parameter_id: GenericParameterId) -> Option<SourceLocation> {
        self.locations_by_parameter.get(&parameter_id).cloned()
    }

    fn record_first_bindings(
        &mut self,
        template: &GenericFunctionTemplate,
        bindings: &GenericTypeBindings,
        type_environment: &TypeEnvironment,
        location: SourceLocation,
    ) {
        let Some(parameter_list) =
            type_environment.generic_parameters(template.generic_parameter_list_id)
        else {
            return;
        };

        for parameter in &parameter_list.parameters {
            if bindings.get(parameter.id).is_some() {
                self.locations_by_parameter
                    .entry(parameter.id)
                    .or_insert_with(|| location.clone());
            }
        }
    }
}

pub(crate) fn infer_generic_function_call(
    input: GenericFunctionInferenceInput<'_>,
) -> Result<GenericFunctionInference, ExpressionParseError> {
    let GenericFunctionInferenceInput {
        template,
        raw_arguments,
        expected_context,
        call_location,
        type_environment,
        string_table,
    } = input;

    let callee_name = template
        .function_path
        .name_str(string_table)
        .map(|name| name.to_owned())
        .unwrap_or_else(|| String::from("<generic function>"));
    let expectations = expectations_from_user_parameters(&template.signature.parameters);
    let routed_arguments = resolve_call_argument_slots_typed(
        CallDiagnosticContext::function(&callee_name),
        raw_arguments,
        &expectations,
        call_location.clone(),
        string_table,
    )?;

    let mut bindings = GenericTypeBindings::new();
    let mut evidence_locations = GenericBindingEvidenceLocations::new();
    collect_call_argument_bindings(
        template,
        &routed_arguments,
        &expectations,
        &mut bindings,
        &mut evidence_locations,
        type_environment,
        string_table,
    )?;

    if !bindings.is_complete_for(template.generic_parameter_list_id, type_environment)
        && let Some(expected_result_type_ids) = expected_context
            .matching_success_results(template.signature.success_return_type_ids().len())
    {
        collect_expected_result_bindings(
            template,
            expected_result_type_ids,
            &mut bindings,
            &mut evidence_locations,
            type_environment,
            string_table,
            call_location.clone(),
        )?;
    }

    let Some(type_arguments) =
        bindings.concrete_arguments_for(template.generic_parameter_list_id, type_environment)
    else {
        let missing_parameters =
            missing_generic_parameter_names(template, &bindings, type_environment);
        return Err(cannot_infer_generic_function_arguments(
            template.function_path.name(),
            missing_parameters,
            call_location,
        )
        .into());
    };

    let mapping = concrete_argument_mapping(
        template.generic_parameter_list_id,
        &type_arguments,
        type_environment,
    )
    .ok_or_else(|| {
        cannot_infer_generic_function_arguments(
            template.function_path.name(),
            missing_generic_parameter_names(template, &bindings, type_environment),
            call_location.clone(),
        )
    })?;
    let signature = substitute_function_signature(&template.signature, &mapping, type_environment);
    let instance_path = generic_function_instance_path(
        &template.function_path,
        type_arguments.as_ref(),
        string_table,
    );

    Ok(GenericFunctionInference {
        key: GenericFunctionInstanceKey {
            function_path: template.function_path.clone(),
            type_arguments,
        },
        instance_path,
        signature,
    })
}

pub(crate) fn validate_generic_function_bound_evidence(
    template: &GenericFunctionTemplate,
    type_arguments: &[TypeId],
    context: &ScopeContext,
    type_environment: &TypeEnvironment,
    call_location: SourceLocation,
) -> Result<Box<[crate::compiler_frontend::traits::ids::TraitEvidenceId]>, ExpressionParseError> {
    let Some(parameter_list) =
        type_environment.generic_parameters(template.generic_parameter_list_id)
    else {
        return Ok(Box::new([]));
    };

    let trait_environment = context.trait_environment();
    let evidence_environment = context.trait_evidence_environment();

    let mut selected_evidence = Vec::new();
    for (parameter, concrete_type_id) in parameter_list.parameters.iter().zip(type_arguments) {
        for trait_id in &parameter.trait_bounds {
            let trait_is_visible = context.trait_id_is_visible(*trait_id);
            if context.generic_template_validation
                && trait_is_visible
                && generic_parameter_declares_bound(*concrete_type_id, *trait_id, type_environment)
            {
                // A template can forward its own bound into a nested generic request. The
                // concrete instance is reparsed later, where the requester selects the actual
                // evidence identity, so template validation must not manufacture a local row.
                continue;
            }
            let evidence_is_visible = trait_is_visible
                && (generated_evidence_pair_is_selected(
                    *concrete_type_id,
                    *trait_id,
                    type_environment,
                    context.shared.generated_evidence_pairs.as_ref(),
                ) || evidence_target_is_visible(
                    *concrete_type_id,
                    type_environment,
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_source_names),
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_type_alias_names),
                    context
                        .shared
                        .file_visibility
                        .as_deref()
                        .map(|visibility| &visibility.visible_namespace_records),
                    context.shared.resolved_type_aliases.as_deref(),
                ));
            let evidence_id = evidence_is_visible
                .then(|| {
                    evidence_for_type(
                        *concrete_type_id,
                        *trait_id,
                        type_environment,
                        evidence_environment,
                    )
                })
                .flatten();

            if let Some(evidence_id) = evidence_id {
                selected_evidence.push(evidence_id);
                continue;
            }

            let trait_name = trait_environment
                .get(*trait_id)
                .map(|definition| definition.name)
                .unwrap_or(template.function_path.name().unwrap_or(parameter.name));

            return Err(missing_generic_function_trait_evidence(
                template.function_path.name(),
                parameter.name,
                trait_name,
                *concrete_type_id,
                call_location,
            )
            .into());
        }
    }

    Ok(selected_evidence.into_boxed_slice())
}

impl<'a> GenericCallExpectedContext<'a> {
    fn matching_success_results(self, success_return_count: usize) -> Option<&'a [TypeId]> {
        match self {
            GenericCallExpectedContext::ImmediateResult(expected_result_type_ids)
                if expected_result_type_ids.len() == success_return_count =>
            {
                Some(expected_result_type_ids)
            }

            GenericCallExpectedContext::ImmediateResult(_) | GenericCallExpectedContext::None => {
                None
            }
        }
    }
}

fn collect_call_argument_bindings(
    template: &GenericFunctionTemplate,
    routed_arguments: &[Option<CallArgument>],
    expectations: &[ParameterExpectation],
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
) -> Result<(), ExpressionParseError> {
    let mut evidence_context = GenericBindingEvidenceContext {
        template,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
    };

    for (slot, argument) in routed_arguments.iter().enumerate() {
        let Some(argument) = argument else {
            continue;
        };
        let ExpectedParameterType::Known(template_type_id) = expectations[slot].expected_type
        else {
            continue;
        };

        collect_binding_evidence(
            &mut evidence_context,
            template_type_id,
            argument.value.type_id,
            argument.location.clone(),
        )?;
    }

    Ok(())
}

fn collect_expected_result_bindings(
    template: &GenericFunctionTemplate,
    expected_result_type_ids: &[TypeId],
    bindings: &mut GenericTypeBindings,
    evidence_locations: &mut GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    let mut evidence_context = GenericBindingEvidenceContext {
        template,
        bindings,
        evidence_locations,
        type_environment,
        string_table,
    };

    for (template_return_type, expected_type) in template
        .signature
        .success_return_type_ids()
        .iter()
        .zip(expected_result_type_ids.iter())
    {
        collect_binding_evidence(
            &mut evidence_context,
            *template_return_type,
            *expected_type,
            location.clone(),
        )?;
    }

    Ok(())
}

fn collect_binding_evidence(
    context: &mut GenericBindingEvidenceContext<'_>,
    template_type_id: TypeId,
    concrete_type_id: TypeId,
    location: SourceLocation,
) -> Result<(), ExpressionParseError> {
    match context
        .type_environment
        .try_collect_type_parameter_bindings_typeid(
            template_type_id,
            concrete_type_id,
            &mut *context.bindings,
        ) {
        Ok(true) => {
            context.evidence_locations.record_first_bindings(
                context.template,
                &*context.bindings,
                context.type_environment,
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
        Err(conflict) => Err(binding_conflict_diagnostic(
            context.template,
            conflict,
            &*context.evidence_locations,
            context.type_environment,
            &mut *context.string_table,
            location,
        )
        .into()),
    }
}

fn binding_conflict_diagnostic(
    template: &GenericFunctionTemplate,
    conflict: BindingConflict,
    evidence_locations: &GenericBindingEvidenceLocations,
    type_environment: &TypeEnvironment,
    string_table: &mut StringTable,
    location: SourceLocation,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let parameter_name = type_environment
        .generic_parameters(template.generic_parameter_list_id)
        .and_then(|list| {
            list.parameters
                .iter()
                .find(|parameter| parameter.id == conflict.parameter_id)
                .map(|parameter| parameter.name)
        })
        .unwrap_or_else(|| string_table.intern("<generic parameter>"));

    conflicting_generic_function_argument(
        template.function_path.name(),
        conflict,
        parameter_name,
        location,
        evidence_locations.previous_location(conflict.parameter_id),
    )
}

pub(crate) fn substitute_function_signature(
    signature: &FunctionSignature,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> FunctionSignature {
    FunctionSignature {
        parameters: signature
            .parameters
            .iter()
            .map(|parameter| substitute_declaration(parameter, mapping, type_environment))
            .collect(),
        returns: signature
            .returns
            .iter()
            .map(|slot| substitute_return_slot(slot, mapping, type_environment))
            .collect(),
    }
}

fn substitute_declaration(
    declaration: &Declaration,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> Declaration {
    let mut declaration = declaration.clone();
    let type_id = type_environment.substitute_type_id(declaration.value.type_id, mapping);
    declaration.value.type_id = type_id;
    declaration.value.diagnostic_type = diagnostic_type_spelling(type_id, type_environment);
    declaration
}

fn substitute_return_slot(
    slot: &ReturnSlot,
    mapping: &FxHashMap<GenericParameterId, TypeId>,
    type_environment: &mut TypeEnvironment,
) -> ReturnSlot {
    let Some(type_id) = slot
        .type_id
        .map(|type_id| type_environment.substitute_type_id(type_id, mapping))
    else {
        return slot.clone();
    };

    let data_type = diagnostic_type_spelling(type_id, type_environment);
    let value = data_type;

    ReturnSlot {
        value,
        type_id: Some(type_id),
        reactive_template: slot.reactive_template.clone(),
        channel: slot.channel,
    }
}

pub(crate) fn concrete_argument_mapping(
    parameter_list_id: GenericParameterListId,
    arguments: &[TypeId],
    type_environment: &TypeEnvironment,
) -> Option<FxHashMap<GenericParameterId, TypeId>> {
    let parameters = type_environment.generic_parameters(parameter_list_id)?;
    if parameters.parameters.len() != arguments.len() {
        return None;
    }

    let mut mapping = FxHashMap::default();
    for (parameter, argument) in parameters.parameters.iter().zip(arguments.iter()) {
        mapping.insert(parameter.id, *argument);
    }

    Some(mapping)
}

fn missing_generic_parameter_names(
    template: &GenericFunctionTemplate,
    bindings: &GenericTypeBindings,
    type_environment: &TypeEnvironment,
) -> Vec<StringId> {
    type_environment
        .generic_parameters(template.generic_parameter_list_id)
        .map(|list| {
            list.parameters
                .iter()
                .filter(|parameter| bindings.get(parameter.id).is_none())
                .map(|parameter| parameter.name)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn generic_function_instance_path(
    function_path: &InternedPath,
    type_arguments: &[TypeId],
    string_table: &mut StringTable,
) -> InternedPath {
    // WHAT: builds an internal-only path for a concrete generic function instance.
    // WHY: instances are emitted into the consuming module's AST/HIR and are not
    //      user-visible, importable, or namespace-exposed.
    //
    // The suffix uses module-local numeric TypeId values. This is safe because:
    // - instances are scoped to one module and never shared across modules;
    // - TypeIds are deterministic within a module;
    // - diagnostics and namespace records always use the authored template name,
    //   not this synthetic path.
    //
    // If backend output or debug symbols ever need stable cross-module names,
    // this suffix can be replaced with a deterministic hash over canonical
    // declaration identity plus TypeEnvironment::type_id_to_type_identity_key.
    let argument_suffix = type_arguments
        .iter()
        .map(|type_id| type_id.0.to_string())
        .collect::<Vec<_>>()
        .join("_");
    function_path.join_str(
        &format!("__generic_instance_{argument_suffix}"),
        string_table,
    )
}
