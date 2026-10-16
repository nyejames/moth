//! Choice constructor expression parsing.
//!
//! WHAT: parses `Choice::Variant` (unit) and `Choice::Variant(...)` (payload) expressions. A
//! payload constructor whose arguments depend on pending numeric origins suspends as one retained
//! call; a generic one also links its declared self shape then. Every retained call resumes
//! through the same finisher at the body root.
//! WHY: choice construction has distinct rules from function calls and struct constructors;
//!      unit variants use value syntax while payload variants use constructor-call syntax.

use std::borrow::Cow;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::binding::resolve_const_expression_content;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ParameterExpectation,
    expectations_from_constructor_fields, expectations_from_constructor_fields_with_substitution,
    resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::constructor_views::{
    ConstructorField, ParsedConstructorExpression,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::ChoiceConstructInput;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ConstructorField as CompletedConstructorField, ExpressionOperationView,
    OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::generic_nominal_inference::{
    GenericNominalConstructorContext, GenericNominalConstructorInput,
    GenericNominalConstructorSeed, GenericNominalTemplate, infer_generic_nominal_constructor,
    seed_generic_nominal_constructor,
};
use crate::compiler_frontend::ast::expressions::generic_nominal_retention::{
    DeferredGenericNominalCall, GenericNominalConstructor,
};
use crate::compiler_frontend::ast::generic_functions::numeric_origins::{
    GenericNominalShapeInput, link_generic_nominal_result_shape,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredExpressionCompletion, DeferredExpressionServices,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::trait_keyword_diagnostics::{
    reserved_trait_keyword_error, reserved_trait_keyword_or_dispatch_mismatch_for_tag,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticToken,
    InvalidChoiceVariantReason,
};
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::declaration_syntax::choice::{ChoiceVariant, ChoiceVariantPayload};
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

/// Parse a `Choice::Variant` or `Choice::Variant(...)` construct expression.
///
/// WHAT: resolves the variant name and validates unit-vs-payload syntax rules.
/// - Unit variants: `Choice::Variant` (no parentheses).
/// - Payload variants: `Choice::Variant(...)` with positional/named arguments.
/// - A payload constructor with numeric-dependent arguments returns one retained operand.
///
/// WHY: the caller has already verified the base symbol is a choice declaration
/// and that `::` follows it.
#[allow(
    clippy::too_many_arguments,
    reason = "choice construction keeps the token stream, declaration, scope, type interner, expression builder, const store, path fork, and string table as separate borrows"
)]
pub(super) fn parse_choice_construct(
    token_stream: &mut AstCursor,
    choice_declaration: &Declaration,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<ParsedConstructorExpression, ExpressionParseError> {
    let type_id = choice_declaration.type_id;

    // The declaration must carry a nominal path so we can look up variant definitions
    // and produce diagnostic type information.
    let Some(nominal_path) = type_interner.environment().nominal_path(type_id) else {
        return Err(CompilerError::compiler_error(
            "Choice construct parser was called with a declaration that has no nominal path.",
        )
        .into());
    };
    let nominal_path = nominal_path.to_owned();
    let generic_declaration_kind = context
        .generic_declarations_by_path
        .as_ref()
        .and_then(|declarations| declarations.get(&nominal_path))
        .filter(|kind| matches!(kind, GenericDeclarationKind::Choice));

    let mut variant_definitions = Some(resolve_choice_variant_definitions(
        &nominal_path,
        type_id,
        context,
        type_interner,
    ));
    let choice_diagnostic_name = CallDiagnosticName::PathComponent {
        component: path_fork.component(nominal_path),
        fallback: "<choice>",
    };

    token_stream.advance();
    if token_stream.current_tag() != TokenTag::DOUBLE_COLON {
        let choice_name = choice_diagnostic_name.rendered_callee_name(string_table);
        return Err(CompilerError::compiler_error(format!(
            "Choice construct parser expected '::' after choice name '{}'.",
            choice_name
        ))
        .into());
    }

    if let Some(gap) = token_stream.current_choice_variant_separator_gap() {
        return Err(CompilerDiagnostic::invalid_choice_variant_spacing(
            gap,
            Some(token_stream.current_span()),
        )
        .into());
    }

    token_stream.advance();

    let variant_span = Some(token_stream.current_span());
    let variant_name = match token_stream.current_tag() {
        TokenTag::SYMBOL => token_stream
            .current_string_id_in(string_table)?
            .ok_or_else(|| {
                CompilerError::compiler_error("choice variant symbol had no string payload")
            })?,

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            let keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                Some(token_stream.current_span()),
                "Expression Parsing",
                "choice variant expression parsing",
            )?;

            return Err(
                reserved_trait_keyword_error(keyword, Some(token_stream.current_span())).into(),
            );
        }

        _ => {
            let found_token = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "choice variant diagnostic",
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            return Err(CompilerDiagnostic::unexpected_token_from_tag(
                found_token,
                Some(token_stream.current_span()),
            )
            .into());
        }
    };

    let Some(variant_index) = variant_definitions
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .position(|variant| variant.name == variant_name)
    else {
        let available_variant_ids: Vec<_> = variant_definitions
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|variant| variant.name)
            .collect();

        return Err(CompilerDiagnostic::invalid_choice_variant(
            InvalidChoiceVariantReason::UnknownVariant,
            path_fork.component(choice_declaration.id),
            Some(variant_name),
            available_variant_ids,
            variant_span,
        )
        .into());
    };

    // `variant_index` came from a position search over this same immutable variant slice.
    let variant = variant_definitions.as_deref().unwrap_or(&[])[variant_index].clone();
    let mut constructor_span = variant_span;
    let has_parens = token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS);
    let mut parsed_payload_arguments = None;
    let mut parsed_payload_expectations = None;
    let mut generic_constructor_seed = None;
    // Unit variants and payloads without parentheses still need contextual bindings before
    // generic inference. Seed here so the variant-table borrow ends before inference mutates types.
    if generic_declaration_kind.is_some()
        && !(matches!(
            &variant.payload,
            ChoiceVariantPayloadDefinition::Record { .. }
        ) && has_parens)
    {
        generic_constructor_seed = Some(seed_generic_nominal_constructor(
            GenericNominalConstructorContext {
                nominal_path: &nominal_path,
                diagnostic_name: choice_diagnostic_name,
                template: GenericNominalTemplate::ChoiceVariants(
                    variant_definitions.as_deref().unwrap_or(&[]),
                ),
                span: constructor_span,
            },
            context,
            type_interner.environment(),
            string_table,
        )?);
        drop(variant_definitions.take());
    } else if generic_declaration_kind.is_none() {
        // Non-generic construction keeps only the selected variant, not this complete table.
        drop(variant_definitions.take());
    }

    // Contextual substitutions make bound generic payload fields immediate literal receivers.
    // Unbound parameter fields keep the parser's inference expectation, so their numeric origins
    // stay pending until the instance is known.
    if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload
        && has_parens
    {
        token_stream.advance(); // past variant name to '('
        constructor_span = Some(token_stream.current_span());

        let payload_field_views = ConstructorField::from_choice_payload_fields(fields);
        let mut field_expectations =
            expectations_from_constructor_fields(&payload_field_views, path_fork);

        if generic_declaration_kind.is_some() {
            let seed = seed_generic_nominal_constructor(
                GenericNominalConstructorContext {
                    nominal_path: &nominal_path,
                    diagnostic_name: choice_diagnostic_name,
                    template: GenericNominalTemplate::ChoiceVariants(
                        variant_definitions.as_deref().unwrap_or(&[]),
                    ),
                    span: constructor_span,
                },
                context,
                type_interner.environment(),
                string_table,
            )?;
            // The seed owns contextual evidence, so release the borrowed table before substitution
            // mutates the type environment.
            drop(variant_definitions.take());
            seed.bind_parse_expectations(
                &mut field_expectations,
                type_interner.environment_mut_for_derived_types(),
            );
            generic_constructor_seed = Some(seed);
        }

        // A payload field whose type is still a generic parameter has no numeric type yet, so its
        // numeric argument stays pending. A concrete payload field retains the same way, because
        // its type is already resolved and the constructor suspends as one operand.
        let callee_name = CallDiagnosticName::ChoiceVariant {
            choice_name: path_fork.component(nominal_path),
            variant_name,
        };
        match parse_call_arguments_typed_with_numeric_retention(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            &field_expectations,
            CallArgumentSyntax::Constructor { callee_name },
            path_fork,
        )? {
            ParsedCallArguments::Ready(arguments) => match generic_constructor_seed.take() {
                // A generic payload selects its instance here, through the completion the retained
                // lane also resumes. Unit and malformed syntax keep their own rules below.
                Some(seed) => {
                    let completion = GenericChoiceCompletion {
                        nominal_path: &nominal_path,
                        diagnostic_name: choice_diagnostic_name,
                        // The seed holds expected-result evidence; inference needs only this payload.
                        template_fields: fields,
                        payload_field_views: &payload_field_views,
                        variant_index,
                        variant_name,
                        constructor_span,
                        variant_span,
                        enforce_constant_context: context.kind.allows_const_record_coercion(),
                        seed,
                        arguments,
                    };
                    let expression = complete_generic_choice_instance(
                        completion,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        path_fork,
                        string_table,
                    )?;
                    return Ok(ParsedConstructorExpression::Value(expression));
                }
                None => {
                    parsed_payload_expectations = Some(field_expectations);
                    parsed_payload_arguments = Some(arguments);
                }
            },
            ParsedCallArguments::Retained(arguments) => {
                let operand = match generic_constructor_seed.take() {
                    Some(seed) => {
                        let result_shape =
                            link_generic_nominal_result_shape(GenericNominalShapeInput {
                                nominal_path: &nominal_path,
                                diagnostic_name: choice_diagnostic_name,
                                declared_fields: &payload_field_views,
                                arguments: &arguments,
                                seeded_bindings: seed.bindings(),
                                context,
                                type_interner,
                                expression_builder,
                                string_table,
                                call_span: constructor_span,
                            })?;
                        DeferredGenericNominalCall {
                            nominal_path,
                            diagnostic_name: choice_diagnostic_name,
                            seed,
                            arguments,
                            result_shape,
                            call_span: constructor_span,
                            constructor: GenericNominalConstructor::Choice {
                                variant,
                                payload_field_views,
                                variant_index,
                                variant_span,
                                enforce_constant_context: context
                                    .kind
                                    .allows_const_record_coercion(),
                            },
                        }
                        .into_pending_operand()
                    }
                    None => ExpressionRpnItem::PendingNumericCall {
                        call: Box::new(DeferredCall::Choice(Box::new(DeferredChoiceCall {
                            nominal_path,
                            choice_type_id: type_id,
                            variant_index,
                            variant_name,
                            expectations: field_expectations,
                            arguments,
                            call_span: constructor_span,
                            variant_span,
                            enforce_constant_context: context.kind.allows_const_record_coercion(),
                        }))),
                        span: constructor_span,
                    },
                };
                return Ok(ParsedConstructorExpression::RetainedOperand(operand));
            }
        }
    }

    // ---------------------------
    //  Resolve generic parameters
    // ---------------------------
    // A generic payload resolved its instance at argument parse, so only unit and malformed record
    // syntax reaches here with a seed. That syntax has no raw arguments to infer from.
    let choice_type_id = if let Some(seed) = generic_constructor_seed {
        let constructor_fields = match &variant.payload {
            ChoiceVariantPayloadDefinition::Record { fields } => {
                Some(ConstructorField::from_choice_payload_fields(fields))
            }
            ChoiceVariantPayloadDefinition::Unit => None,
        };
        let inference = infer_generic_nominal_constructor(
            GenericNominalConstructorInput {
                // Context-derived bindings were captured by the seed before this inference step.
                context: GenericNominalConstructorContext {
                    nominal_path: &nominal_path,
                    diagnostic_name: choice_diagnostic_name,
                    template: GenericNominalTemplate::ChoiceVariants(&[]),
                    span: constructor_span,
                },
                constructor_fields: constructor_fields.as_deref(),
                raw_args: None,
                expression_builder,
            },
            seed,
            context,
            type_interner,
            string_table,
        )?;
        inference.instance_type_id
    } else {
        type_id
    };

    // ----------------------------------------
    //  Build unit or record choice expression
    // ----------------------------------------
    match &variant.payload {
        ChoiceVariantPayloadDefinition::Unit => {
            token_stream.advance();

            if has_parens {
                token_stream.advance(); // past '('

                if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
                    return Err(CompilerDiagnostic::invalid_choice_variant(
                        InvalidChoiceVariantReason::UnitVariantWithParentheses,
                        path_fork.component(choice_declaration.id),
                        Some(variant_name),
                        vec![],
                        Some(token_stream.current_span()),
                    )
                    .into());
                }

                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::UnitVariantAsConstructor,
                    path_fork.component(choice_declaration.id),
                    Some(variant_name),
                    vec![],
                    variant_span,
                )
                .into());
            }

            let choice_expr = expression_builder.choice_construct(ChoiceConstructInput {
                tag: variant_index,
                fields: vec![],
                type_id: choice_type_id,
                span: variant_span,
                value_mode: ValueMode::ImmutableOwned,
            })?;
            Ok(ParsedConstructorExpression::Value(choice_expr))
        }

        ChoiceVariantPayloadDefinition::Record { fields } => {
            if !has_parens {
                token_stream.advance();
                return Err(CompilerDiagnostic::invalid_choice_variant(
                    InvalidChoiceVariantReason::PayloadVariantMissingArguments,
                    path_fork.component(choice_declaration.id),
                    Some(variant_name),
                    vec![],
                    Some(token_stream.current_span()),
                )
                .into());
            }

            // This should always be Some because record variants with parentheses
            // are pre-parsed above. A missing value indicates an internal invariant.
            let Some(raw_args) = parsed_payload_arguments.take() else {
                return Err(CompilerError::compiler_error(
                    "Payload choice constructor reached validation without parsed arguments.",
                )
                .into());
            };
            let Some(expectations) = parsed_payload_expectations.take() else {
                return Err(CompilerError::compiler_error(
                    "Payload choice constructor reached validation without parsed expectations.",
                )
                .into());
            };
            let choice_expr = finish_choice_payload_construct(
                ChoicePayloadConstructInput {
                    diagnostic_name: CallDiagnosticName::ChoiceVariant {
                        choice_name: path_fork.component(nominal_path),
                        variant_name,
                    },
                    expectations: &expectations,
                    variant_index,
                    choice_type_id,
                    fields,
                    constructor_span,
                    variant_span,
                    enforce_constant_context: context.kind.allows_const_record_coercion(),
                },
                raw_args,
                context,
                type_interner,
                expression_builder,
                const_values,
                path_fork,
                string_table,
            )?;

            Ok(ParsedConstructorExpression::Value(choice_expr))
        }
    }
}

/// Resolve the canonical variant definitions of one choice type.
///
/// WHAT: prefers the populated `TypeEnvironment` and falls back to the header-stage shell
///       declarations when the environment has not been populated yet.
/// WHY: one resolution rule serves both the direct constructor and its retained completion, so a
///      suspended constructor re-derives the same variant list instead of keeping a second copy.
fn resolve_choice_variant_definitions<'a>(
    nominal_path: &PathId,
    type_id: TypeId,
    context: &ScopeContext,
    type_interner: &'a AstTypeInterner<'_>,
) -> Cow<'a, [ChoiceVariantDefinition]> {
    if let Some(definitions) = type_interner
        .environment()
        .variants_for(type_id)
        .filter(|definitions| !definitions.is_empty())
    {
        return Cow::Borrowed(definitions);
    }

    context
        .choice_variant_shells_by_path
        .as_ref()
        .and_then(|shells| shells.get(nominal_path))
        .map(|shells| Cow::Owned(choice_variant_shells_to_definitions(shells)))
        .unwrap_or(Cow::Borrowed(&[]))
}

/// Resolved payload-constructor facts shared by the direct and retained completion paths.
struct ChoicePayloadConstructInput<'a> {
    diagnostic_name: CallDiagnosticName<'static>,
    expectations: &'a [ParameterExpectation],
    variant_index: usize,
    choice_type_id: TypeId,
    fields: &'a [FieldDefinition],
    constructor_span: Option<SourceSpan>,
    variant_span: Option<SourceSpan>,
    enforce_constant_context: bool,
}

/// Validate one record payload constructor's arguments and build the canonical choice value.
///
/// WHAT: the single construction tail for both the direct path and the retained-call resumption:
///       argument compatibility, const-context payload checks and definition-order payload fields.
/// WHY: a retained constructor must resume through the exact ordinary construction rules, so the
///      two lanes share one finisher instead of a second validator that could drift.
#[allow(
    clippy::too_many_arguments,
    reason = "choice payload finishing keeps the input, owned arguments, scope, type interner, expression builder, const store, path fork, and string table as separate inputs"
)]
fn finish_choice_payload_construct(
    input: ChoicePayloadConstructInput<'_>,
    raw_args: Vec<CallArgument<OwnedExpr>>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<OwnedExpr, ExpressionParseError> {
    let ChoicePayloadConstructInput {
        diagnostic_name,
        expectations,
        variant_index,
        choice_type_id,
        fields,
        constructor_span,
        variant_span,
        enforce_constant_context,
    } = input;
    let type_check_context = type_interner.type_check_context();
    let resolved_args = resolve_call_arguments(
        CallDiagnosticContext::choice_constructor(diagnostic_name),
        raw_args,
        expectations,
        constructor_span,
        CallArgumentResolutionContext {
            string_table,
            path_fork,
            type_environment: type_check_context.type_environment,
            expression_builder,
            const_values,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )?;

    let mut choice_fields = Vec::with_capacity(fields.len());

    for (slot, (field, arg)) in fields.iter().zip(resolved_args).enumerate() {
        let mut value = arg.value;

        if enforce_constant_context {
            // Header placeholders remain references until graph-ordered constant resolution.
            let (field_span, value_is_compile_time_constant) = {
                let view = expression_builder.view_owned(value)?;
                let is_placeholder_reference =
                    if let ExpressionOperationView::Reference(path) = view.operation() {
                        path_fork.component(path).is_some_and(|name| {
                            context.get_reference(&name).is_some_and(|reference| {
                                reference.ready_declaration().is_some_and(|declaration| {
                                    context.is_unresolved_constant_placeholder(declaration)
                                })
                            })
                        })
                    } else {
                        false
                    };
                let value_is_compile_time_constant = if is_placeholder_reference {
                    true
                } else {
                    view.const_value_kind_with_template_classifier(const_values, &mut |template| {
                        classify_template_from_effective_tir(
                            template,
                            &context.template_ir_store,
                            expression_builder,
                            const_values,
                        )
                    })?
                    .is_compile_time_value()
                };
                (view.span(), value_is_compile_time_constant)
            };

            if !value_is_compile_time_constant {
                let field_name = path_fork
                    .component(field.name)
                    .unwrap_or_else(|| string_table.intern("<field>"));
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext,
                    Some(field_name),
                    field_span,
                )
                .into());
            }

            let folded_type_id = expression_builder.view_owned(value)?.value_type_id()?;
            let resolved = resolve_const_expression_content(
                value.id(),
                expression_builder,
                const_values,
                type_interner.environment(),
                context,
                string_table,
            )?;
            value = expression_builder.constant(
                resolved,
                folded_type_id,
                field_span,
                ValueMode::ImmutableOwned,
            )?;
        }

        choice_fields.push(CompletedConstructorField {
            name: field.name,
            slot: u32::try_from(slot)
                .map_err(|_| CompilerError::compiler_error("constructor field slot exceeds u32"))?,
            value,
        });
    }

    let value_mode = if enforce_constant_context {
        ValueMode::ImmutableOwned
    } else {
        // Non-const choice construction always produces a mutable owned value
        // because there is no caller-supplied ownership mode for choices.
        ValueMode::MutableOwned
    };

    Ok(expression_builder.choice_construct(ChoiceConstructInput {
        tag: variant_index,
        fields: choice_fields,
        type_id: choice_type_id,
        span: variant_span,
        value_mode,
    })?)
}

/// One concrete choice payload constructor retained until its numeric-dependent arguments resolve.
///
/// WHAT: the resolved choice identity and canonical result type, the selected variant, the intake
///       expectations, the ordered argument list with its authored routing, the real spans, and the
///       const-context policy.
/// WHY: the payload field types are already concrete, so argument compatibility cannot run before
///      the arguments' numeric origins are selected. Completion reuses the intake expectations
///      rather than rebuilding them, while the variant list and payload fields stay with the
///      canonical environment and are not copied into this record.
#[derive(Debug, Clone)]
pub(crate) struct DeferredChoiceCall {
    pub(crate) nominal_path: PathId,
    pub(crate) choice_type_id: TypeId,
    pub(crate) variant_index: usize,
    pub(crate) variant_name: StringId,
    pub(crate) expectations: Vec<ParameterExpectation>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) variant_span: Option<SourceSpan>,
    pub(crate) enforce_constant_context: bool,
}

/// Resume one retained concrete choice payload constructor after its arguments resolve.
///
/// WHAT: completes every retained argument through the session's shared call-argument service,
///       re-derives the selected variant's payload fields from the canonical environment, then
///       builds the canonical choice value through the same finisher the direct path uses.
/// WHY: the constructor was suspended before argument compatibility, so its own receiving
///      boundary is the only place the completed argument types may be checked and the
///      definition-order payload fields may be produced.
pub(crate) fn complete_deferred_choice_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredChoiceCall,
    session: &mut S,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredChoiceCall {
        nominal_path,
        choice_type_id,
        variant_index,
        variant_name,
        expectations,
        arguments,
        call_span,
        variant_span,
        enforce_constant_context,
    } = deferred;

    // Arguments complete before the session services are borrowed: both take the session mutably.
    let mut completed_arguments = Vec::with_capacity(arguments.len());
    for argument in arguments {
        completed_arguments.push(session.complete_call_argument(argument)?);
    }

    let DeferredExpressionServices {
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    } = session.session_services();

    // A retained constructor only exists for a non-generic choice, whose environment entries are
    // populated before its body is emitted; the shared resolution rule still handles shells.
    let variant_definitions =
        resolve_choice_variant_definitions(&nominal_path, choice_type_id, context, type_interner);
    let Some(selected_variant) = variant_definitions.get(variant_index).cloned() else {
        return Err(CompilerError::compiler_error(
            "a retained choice constructor's variant was missing from its resolved variant list",
        )
        .into());
    };
    drop(variant_definitions);
    let ChoiceVariantPayloadDefinition::Record { fields } = &selected_variant.payload else {
        return Err(CompilerError::compiler_error(
            "a retained choice constructor's selected variant was not a payload variant",
        )
        .into());
    };

    finish_choice_payload_construct(
        ChoicePayloadConstructInput {
            diagnostic_name: CallDiagnosticName::ChoiceVariant {
                choice_name: path_fork.component(nominal_path),
                variant_name,
            },
            expectations: &expectations,
            variant_index,
            choice_type_id,
            fields,
            constructor_span: call_span,
            variant_span,
            enforce_constant_context,
        },
        completed_arguments,
        context,
        type_interner,
        expression_builder,
        const_values,
        path_fork,
        string_table,
    )
}

/// Facts shared by immediate and retained generic choice-payload completion.
///
/// WHAT: the selected payload's declared fields and constructor views, its variant identity, the
///       contextual seed, and the owned argument list.
/// WHY: retaining these views across numeric completion avoids rebuilding the call's field
///       projection after the parser already used it for routing and inference.
pub(crate) struct GenericChoiceCompletion<'a> {
    pub(crate) nominal_path: &'a PathId,
    pub(crate) diagnostic_name: CallDiagnosticName<'static>,
    pub(crate) template_fields: &'a [FieldDefinition],
    pub(crate) payload_field_views: &'a [ConstructorField],
    pub(crate) variant_index: usize,
    pub(crate) variant_name: StringId,
    pub(crate) constructor_span: Option<SourceSpan>,
    pub(crate) variant_span: Option<SourceSpan>,
    pub(crate) enforce_constant_context: bool,
    pub(crate) seed: GenericNominalConstructorSeed,
    pub(crate) arguments: Vec<CallArgument<OwnedExpr>>,
}

/// Infer one generic choice payload instance from completed arguments and build its value.
///
/// WHY: the selected instance is known only after argument completion, but the parser's field views
///       remain valid for both inference and final validation without another projection.
pub(crate) fn complete_generic_choice_instance(
    completion: GenericChoiceCompletion<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<OwnedExpr, ExpressionParseError> {
    let GenericChoiceCompletion {
        nominal_path,
        diagnostic_name,
        template_fields,
        payload_field_views,
        variant_index,
        variant_name,
        constructor_span,
        variant_span,
        enforce_constant_context,
        seed,
        arguments,
    } = completion;

    let inference = infer_generic_nominal_constructor(
        GenericNominalConstructorInput {
            context: GenericNominalConstructorContext {
                nominal_path,
                diagnostic_name,
                // Expected-result evidence is already captured in the seed.
                template: GenericNominalTemplate::ChoiceVariants(&[]),
                span: constructor_span,
            },
            constructor_fields: Some(payload_field_views),
            raw_args: Some(&arguments),
            expression_builder,
        },
        seed,
        context,
        type_interner,
        string_table,
    )?;

    let resolved_expectations = expectations_from_constructor_fields_with_substitution(
        payload_field_views,
        path_fork,
        type_interner.environment_mut_for_derived_types(),
        inference.bindings.replacement_map(),
    );
    finish_choice_payload_construct(
        ChoicePayloadConstructInput {
            diagnostic_name: CallDiagnosticName::ChoiceVariant {
                choice_name: path_fork.component(*nominal_path),
                variant_name,
            },
            expectations: &resolved_expectations,
            variant_index,
            choice_type_id: inference.instance_type_id,
            fields: template_fields,
            constructor_span,
            variant_span,
            enforce_constant_context,
        },
        arguments,
        context,
        type_interner,
        expression_builder,
        const_values,
        path_fork,
        string_table,
    )
}

/// Convert AST-owned choice variant shells to `ChoiceVariantDefinition` for early constructor
/// validation.
///
/// WHAT: bridges unresolved `ChoiceVariant` shells (carrying `Declaration` payloads) to the
/// `ChoiceVariantDefinition` shape that call validation and generic inference expect.
/// WHY: choice constructor parsing must work before final canonical variants are written to
/// `TypeEnvironment`. The shell declarations already carry the semantic `TypeId`s produced by
/// AST resolution, so this fallback does not re-run diagnostic type conversion or add
/// `TypeEnvironment` placeholders.
fn choice_variant_shells_to_definitions(shells: &[ChoiceVariant]) -> Vec<ChoiceVariantDefinition> {
    shells
        .iter()
        .enumerate()
        .map(|(tag, variant)| ChoiceVariantDefinition {
            name: variant.id,
            tag,
            payload: match &variant.payload {
                ChoiceVariantPayload::Unit => ChoiceVariantPayloadDefinition::Unit,

                ChoiceVariantPayload::Record { fields } => {
                    let field_defs: Vec<FieldDefinition> = fields
                        .iter()
                        .map(|field| FieldDefinition {
                            name: field.id,
                            type_id: field.type_id,
                            span: field.binding_span,
                        })
                        .collect();

                    ChoiceVariantPayloadDefinition::Record {
                        fields: field_defs.into_boxed_slice(),
                    }
                }
            },
            span: variant.span,
        })
        .collect()
}
