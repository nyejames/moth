//! Choice constructor expression parsing.
//!
//! WHAT: parses `Choice::Variant` (unit) and `Choice::Variant(...)` (payload) expressions. A
//! payload constructor whose arguments depend on pending numeric origins suspends as one retained
//! call; a generic one also links its declared self shape then. Every retained call resumes
//! through the same finisher at the body root.
//! WHY: choice construction has distinct rules from function calls and struct constructors;
//!      unit variants use value syntax while payload variants use constructor-call syntax.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::call_arguments::{
    CallArgumentSyntax, ParsedCallArguments, parse_call_arguments_typed_with_numeric_retention,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, expectations_from_constructor_fields,
    resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::constructor_views::{
    ConstructorField, ParsedConstructorExpression,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    ChoiceConstructInput, Expression, ExpressionKind,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
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
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
};
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey;
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
pub(super) fn parse_choice_construct(
    token_stream: &mut AstCursor,
    choice_declaration: &Declaration,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<ParsedConstructorExpression, ExpressionParseError> {
    let type_id = choice_declaration.value.type_id;

    // The declaration must carry a nominal path so we can look up variant definitions
    // and produce diagnostic type information.
    let Some(nominal_path) = type_interner.environment().nominal_path(type_id) else {
        return Err(CompilerError::compiler_error(
            "Choice construct parser was called with a declaration that has no nominal path.",
        )
        .into());
    };
    let nominal_path = nominal_path.to_owned();

    let variant_definitions =
        resolve_choice_variant_definitions(&nominal_path, type_id, context, type_interner);
    let choice_name_str = choice_display_name(&nominal_path, path_fork, string_table);

    token_stream.advance();
    if token_stream.current_tag() != TokenTag::DOUBLE_COLON {
        return Err(CompilerError::compiler_error(format!(
            "Choice construct parser expected '::' after choice name '{}'.",
            choice_name_str
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
        .iter()
        .position(|variant| variant.name == variant_name)
    else {
        let available_variant_ids: Vec<_> = variant_definitions.iter().map(|v| v.name).collect();

        return Err(CompilerDiagnostic::invalid_choice_variant(
            InvalidChoiceVariantReason::UnknownVariant,
            path_fork.component(choice_declaration.id),
            Some(variant_name),
            available_variant_ids,
            variant_span,
        )
        .into());
    };

    let variant = &variant_definitions[variant_index];
    let has_parens = token_stream.peek_next_tag() == Some(TokenTag::OPEN_PARENTHESIS);
    let mut parsed_payload_arguments = None;
    let mut constructor_span = variant_span;
    let generic_declaration_kind = context
        .generic_declarations_by_path
        .as_ref()
        .and_then(|declarations| declarations.get(&nominal_path))
        .filter(|kind| matches!(kind, GenericDeclarationKind::Choice));
    let mut generic_constructor_context = GenericNominalConstructorContext {
        nominal_path: &nominal_path,
        display_name: &choice_name_str,
        template: GenericNominalTemplate::ChoiceVariants(&variant_definitions),
        span: constructor_span,
    };
    let mut generic_constructor_seed = None;

    // Contextual substitutions make bound generic payload fields immediate literal receivers.
    // Unbound parameter fields keep the parser's inference expectation, so their numeric origins
    // stay pending until the instance is known.
    if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload
        && has_parens
    {
        token_stream.advance(); // past variant name to '('
        constructor_span = Some(token_stream.current_span());
        generic_constructor_context.span = constructor_span;

        let payload_field_views = ConstructorField::from_choice_payload_fields(fields);
        let mut field_expectations =
            expectations_from_constructor_fields(&payload_field_views, path_fork);

        if generic_declaration_kind.is_some() {
            let seed = seed_generic_nominal_constructor(
                generic_constructor_context,
                context,
                type_interner.environment(),
                string_table,
            )?;
            seed.bind_parse_expectations(
                &mut field_expectations,
                type_interner.environment_mut_for_derived_types(),
            );
            generic_constructor_seed = Some(seed);
        }

        // A payload field whose type is still a generic parameter has no numeric type yet, so its
        // numeric argument stays pending. A concrete payload field retains the same way, because
        // its type is already resolved and the constructor suspends as one operand.
        let callee_name = choice_variant_callee_name(&choice_name_str, variant_name, string_table);
        match parse_call_arguments_typed_with_numeric_retention(
            token_stream,
            context,
            type_interner,
            string_table,
            &field_expectations,
            CallArgumentSyntax::Supported {
                callee_name: Some(callee_name),
            },
            path_fork,
        )? {
            ParsedCallArguments::Ready(arguments) => match generic_constructor_seed.take() {
                // A generic payload selects its instance here, through the completion the retained
                // lane also resumes. Unit and malformed syntax keep their own rules below.
                Some(seed) => {
                    let completion = GenericChoiceCompletion {
                        nominal_path: &nominal_path,
                        display_name: &choice_name_str,
                        variants: &variant_definitions,
                        variant_index,
                        variant_name,
                        constructor_span,
                        variant_span,
                        enforce_constant_context: context.kind.allows_const_record_coercion(),
                        seed,
                        arguments: &arguments,
                    };
                    let expression = complete_generic_choice_instance(
                        completion,
                        context,
                        type_interner,
                        path_fork,
                        string_table,
                    )?;
                    return Ok(ParsedConstructorExpression::Value(expression));
                }
                None => parsed_payload_arguments = Some(arguments),
            },
            ParsedCallArguments::Retained(arguments) => {
                let operand = match generic_constructor_seed.take() {
                    Some(seed) => {
                        let result_shape =
                            link_generic_nominal_result_shape(GenericNominalShapeInput {
                                nominal_path: &nominal_path,
                                display_name: &choice_name_str,
                                declared_fields: &payload_field_views,
                                arguments: &arguments,
                                seeded_bindings: seed.bindings(),
                                context,
                                type_interner,
                                string_table,
                                call_span: constructor_span,
                            })?;
                        DeferredGenericNominalCall {
                            nominal_path,
                            display_name: choice_name_str,
                            seed,
                            arguments,
                            result_shape,
                            call_span: constructor_span,
                            constructor: GenericNominalConstructor::Choice {
                                variants: variant_definitions,
                                variant_index,
                                variant_name,
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

    // Unit variants and malformed record syntax have no argument parse to seed from, but
    // still use contextual inference before the selected variant is instantiated.
    if generic_declaration_kind.is_some() && generic_constructor_seed.is_none() {
        generic_constructor_seed = Some(seed_generic_nominal_constructor(
            generic_constructor_context,
            context,
            type_interner.environment(),
            string_table,
        )?);
    }

    // ---------------------------
    //  Resolve generic parameters
    // ---------------------------
    // A generic payload resolved its instance at argument parse, so only unit and malformed record
    // syntax reaches here with a seed. That syntax has no raw arguments to infer from.
    let (choice_type_id, generic_instance_key) = if let Some(seed) = generic_constructor_seed {
        let constructor_fields = match &variant.payload {
            ChoiceVariantPayloadDefinition::Record { fields } => {
                Some(ConstructorField::from_choice_payload_fields(fields))
            }
            ChoiceVariantPayloadDefinition::Unit => None,
        };
        let inference = infer_generic_nominal_constructor(
            GenericNominalConstructorInput {
                context: generic_constructor_context,
                constructor_fields: constructor_fields.as_deref(),
                raw_args: None,
                path_fork,
            },
            seed,
            context,
            type_interner,
            string_table,
        )?;
        (inference.instance_type_id, inference.instance_key)
    } else {
        (type_id, None)
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

            let diagnostic_type = DataType::Choices {
                nominal_path,
                type_id: choice_type_id,
                generic_instance_key,
            };

            let choice_expr = Expression::choice_construct(ChoiceConstructInput {
                nominal_path,
                tag: variant_index,
                fields: vec![],
                diagnostic_type,
                type_id: choice_type_id,
                span: variant_span,
                value_mode: ValueMode::ImmutableOwned,
            });
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
            let Some(raw_args) = parsed_payload_arguments.as_ref() else {
                return Err(CompilerError::compiler_error(
                    "Payload choice constructor reached validation without parsed arguments.",
                )
                .into());
            };

            let choice_expr = finish_choice_payload_construct(
                ChoicePayloadConstructInput {
                    nominal_path: &nominal_path,
                    choice_name_display: &choice_name_str,
                    variant_name,
                    variant_index,
                    choice_type_id,
                    generic_instance_key,
                    fields,
                    constructor_span,
                    variant_span,
                    enforce_constant_context: context.kind.allows_const_record_coercion(),
                },
                raw_args,
                context,
                type_interner,
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
fn resolve_choice_variant_definitions(
    nominal_path: &PathId,
    type_id: TypeId,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
) -> Vec<ChoiceVariantDefinition> {
    type_interner
        .environment()
        .variants_for(type_id)
        .filter(|defs| !defs.is_empty())
        .map(|defs| defs.to_vec())
        .or_else(|| {
            context
                .choice_variant_shells_by_path
                .as_ref()
                .and_then(|shells| shells.get(nominal_path))
                .map(|shells| choice_variant_shells_to_definitions(shells))
        })
        .unwrap_or_default()
}

/// Visible display name of one choice type, used by constructor diagnostics.
fn choice_display_name(
    nominal_path: &PathId,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
) -> String {
    path_fork
        .component(*nominal_path)
        .map(|name| string_table.resolve(name))
        .unwrap_or("<choice>")
        .to_owned()
}

/// The diagnostic callee name of one payload constructor (`Choice::Variant`).
fn choice_variant_callee_name(
    choice_name_display: &str,
    variant_name: StringId,
    string_table: &mut StringTable,
) -> StringId {
    let variant_name_str = string_table.resolve(variant_name).to_owned();
    string_table.intern(&format!("{choice_name_display}::{variant_name_str}"))
}

/// Resolved payload-constructor facts shared by the direct and retained completion paths.
struct ChoicePayloadConstructInput<'a> {
    nominal_path: &'a PathId,
    choice_name_display: &'a str,
    variant_name: StringId,
    variant_index: usize,
    choice_type_id: TypeId,
    generic_instance_key: Option<GenericInstantiationKey>,
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
fn finish_choice_payload_construct(
    input: ChoicePayloadConstructInput<'_>,
    raw_args: &[CallArgument],
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let ChoicePayloadConstructInput {
        nominal_path,
        choice_name_display,
        variant_name,
        variant_index,
        choice_type_id,
        generic_instance_key,
        fields,
        constructor_span,
        variant_span,
        enforce_constant_context,
    } = input;

    let constructor_fields = ConstructorField::from_choice_payload_fields(fields);
    let expectations = expectations_from_constructor_fields(&constructor_fields, path_fork);
    let type_check_context = type_interner.type_check_context();
    let resolved_args = resolve_call_arguments(
        CallDiagnosticContext::choice_constructor(&format!(
            "{}::{}",
            choice_name_display,
            string_table.resolve(variant_name)
        )),
        raw_args,
        &expectations,
        constructor_span,
        CallArgumentResolutionContext {
            string_table,
            path_fork,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )?;

    let mut choice_fields = Vec::with_capacity(fields.len());

    for (field, arg) in fields.iter().zip(resolved_args) {
        let mut value = arg.value;

        if enforce_constant_context {
            // Header-stage choice shells may carry placeholder references until AST
            // environment construction resolves constants in graph order.
            let is_placeholder_reference = if let ExpressionKind::Reference(path) = &value.kind {
                path_fork.component(*path).is_some_and(|name| {
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
                value
                    .const_value_kind_with_template_classifier(&mut |template| {
                        classify_template_from_effective_tir(template, &context.template_ir_store)
                    })?
                    .is_compile_time_value()
            };

            if !value_is_compile_time_constant {
                let field_name = path_fork
                    .component(field.name)
                    .unwrap_or_else(|| string_table.intern("<field>"));
                return Err(CompilerDiagnostic::compile_time_evaluation_error(
                    CompileTimeEvaluationErrorReason::NonCompileTimeFieldInConstantContext,
                    Some(field_name),
                    value.span,
                )
                .into());
            }

            // Const records are data-only exports, so mutable ownership is removed to
            // keep constant semantics explicit in later stages.
            value.value_mode = ValueMode::ImmutableOwned;
        }

        choice_fields.push(Declaration {
            id: field.name,
            value,
            binding_span: None,
            config_qualifier: None,
        });
    }

    let value_mode = if enforce_constant_context {
        ValueMode::ImmutableOwned
    } else {
        // Non-const choice construction always produces a mutable owned value
        // because there is no caller-supplied ownership mode for choices.
        ValueMode::MutableOwned
    };

    let diagnostic_type = DataType::Choices {
        nominal_path: *nominal_path,
        type_id: choice_type_id,
        generic_instance_key,
    };

    Ok(Expression::choice_construct(ChoiceConstructInput {
        nominal_path: *nominal_path,
        tag: variant_index,
        fields: choice_fields,
        diagnostic_type,
        type_id: choice_type_id,
        span: variant_span,
        value_mode,
    }))
}

/// One concrete choice payload constructor retained until its numeric-dependent arguments resolve.
///
/// WHAT: the resolved choice identity and canonical result type, the selected variant, the ordered
///       argument list with its authored routing, the real spans, and the const-context policy.
/// WHY: the payload field types are already concrete, so argument compatibility cannot run before
///      the arguments' numeric origins are selected. The variant list and payload fields are
///      re-derived from the canonical environment at completion through the same resolution rule
///      the direct path uses, so no second copy of them is retained.
#[derive(Debug, Clone)]
pub(crate) struct DeferredChoiceCall {
    pub(crate) nominal_path: PathId,
    pub(crate) choice_type_id: TypeId,
    pub(crate) variant_index: usize,
    pub(crate) variant_name: StringId,
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
) -> Result<Expression, ExpressionParseError> {
    let DeferredChoiceCall {
        nominal_path,
        choice_type_id,
        variant_index,
        variant_name,
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
        string_table,
        path_fork,
    } = session.session_services();

    // A retained constructor only exists for a non-generic choice, whose environment entries are
    // populated before its body is emitted; the shared resolution rule still handles shells.
    let variant_definitions =
        resolve_choice_variant_definitions(&nominal_path, choice_type_id, context, type_interner);
    let Some(selected_variant) = variant_definitions.get(variant_index) else {
        return Err(CompilerError::compiler_error(
            "a retained choice constructor's variant was missing from its resolved variant list",
        )
        .into());
    };
    let ChoiceVariantPayloadDefinition::Record { fields } = &selected_variant.payload else {
        return Err(CompilerError::compiler_error(
            "a retained choice constructor's selected variant was not a payload variant",
        )
        .into());
    };

    let choice_name_str = choice_display_name(&nominal_path, path_fork, string_table);
    finish_choice_payload_construct(
        ChoicePayloadConstructInput {
            nominal_path: &nominal_path,
            choice_name_display: &choice_name_str,
            variant_name,
            variant_index,
            choice_type_id,
            generic_instance_key: None,
            fields,
            constructor_span: call_span,
            variant_span,
            enforce_constant_context,
        },
        &completed_arguments,
        context,
        type_interner,
        path_fork,
        string_table,
    )
}

/// Generic choice payload constructor facts shared by the immediate and retained completion paths.
///
/// WHAT: the nominal identity, the declared variant templates, the selected variant, the contextual
///       seed, and the completed payload arguments.
/// WHY: a generic choice instance is inferred from completed payload types, so both lanes reach the
///      same inference and finisher with the same facts.
pub(crate) struct GenericChoiceCompletion<'a> {
    pub(crate) nominal_path: &'a PathId,
    pub(crate) display_name: &'a str,
    pub(crate) variants: &'a [ChoiceVariantDefinition],
    pub(crate) variant_index: usize,
    pub(crate) variant_name: StringId,
    pub(crate) constructor_span: Option<SourceSpan>,
    pub(crate) variant_span: Option<SourceSpan>,
    pub(crate) enforce_constant_context: bool,
    pub(crate) seed: GenericNominalConstructorSeed,
    pub(crate) arguments: &'a [CallArgument],
}

/// Infer one generic choice payload instance from completed arguments and build its value.
///
/// WHAT: infers the instance from the seed, the selected variant's payload views and the argument
///       types, substitutes the selected declared payload, then runs the shared payload finisher.
/// WHY: the instance is selected only after every payload argument is concrete, so the immediate
///      and retained lanes both end at this one completion.
pub(crate) fn complete_generic_choice_instance(
    completion: GenericChoiceCompletion<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let GenericChoiceCompletion {
        nominal_path,
        display_name,
        variants,
        variant_index,
        variant_name,
        constructor_span,
        variant_span,
        enforce_constant_context,
        seed,
        arguments,
    } = completion;

    let Some(template_variant) = variants.get(variant_index) else {
        return Err(CompilerError::compiler_error(
            "a retained generic choice constructor's variant was missing from its template",
        )
        .into());
    };
    let ChoiceVariantPayloadDefinition::Record {
        fields: template_fields,
    } = &template_variant.payload
    else {
        return Err(CompilerError::compiler_error(
            "a retained generic choice constructor's selected variant was not a payload variant",
        )
        .into());
    };
    let payload_field_views = ConstructorField::from_choice_payload_fields(template_fields);

    let inference = infer_generic_nominal_constructor(
        GenericNominalConstructorInput {
            context: GenericNominalConstructorContext {
                nominal_path,
                display_name,
                template: GenericNominalTemplate::ChoiceVariants(variants),
                span: constructor_span,
            },
            constructor_fields: Some(&payload_field_views),
            raw_args: Some(arguments),
            path_fork,
        },
        seed,
        context,
        type_interner,
        string_table,
    )?;

    // Substitute the declared payload with the inferred arguments. The instance's cached view is not
    // read: an early header-stage completion can run before the base nominal's views are populated.
    let selected_fields: Vec<FieldDefinition> = template_fields
        .iter()
        .map(|field| FieldDefinition {
            name: field.name,
            type_id: type_interner
                .environment_mut_for_derived_types()
                .substitute_type_id(field.type_id, inference.bindings.replacement_map()),
            span: field.span,
        })
        .collect();

    finish_choice_payload_construct(
        ChoicePayloadConstructInput {
            nominal_path,
            choice_name_display: display_name,
            variant_name,
            variant_index,
            choice_type_id: inference.instance_type_id,
            generic_instance_key: inference.instance_key,
            fields: selected_fields.as_slice(),
            constructor_span,
            variant_span,
            enforce_constant_context,
        },
        arguments,
        context,
        type_interner,
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
                            type_id: field.value.type_id,
                            span: field.value.span,
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
