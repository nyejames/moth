//! Struct constructor expression parsing.
//!
//! WHAT: parses `StructName(...)` call-like syntax and lowers it to a typed
//! `Expression::struct_instance` with definition-order fields, generic inference,
//! default filling, and const-context coercion. A constructor whose arguments depend on pending
//! numeric origins suspends as one retained call; a generic one also links its declared self shape
//! then, and every retained call resumes through the same finisher at the body root.
//! WHY: struct constructors share the same argument-resolution pipeline as function
//! calls, so keeping the struct-specific wrapper close to the generic nominal
//! inference and call-validation modules makes the shared machinery easy to follow.

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
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
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
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic,
};
use crate::compiler_frontend::datatypes::generic_identity_bridge::GenericInstantiationKey;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::headers::module_symbols::GenericDeclarationKind;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::value_mode::ValueMode;

/// Input bundle for `parse_struct_constructor_expression`.
///
/// WHAT: carries the struct path, field declarations, value mode, and base type id so
/// the caller (identifier expression dispatch) can hand off a fully-resolved struct
/// identity without repeating lookup work.
pub(crate) struct StructConstructorParseInput<'a> {
    pub(crate) struct_path: &'a PathId,
    pub(crate) struct_name: StringId,
    pub(crate) fields: &'a [Declaration],
    pub(crate) struct_value_mode: &'a ValueMode,
    pub(crate) type_id: TypeId,
}

/// Parse `StructName(...)` and return a finished value or one retained constructor operand.
///
/// WHAT:
/// - Parses constructor arguments (positional and named) using the shared call-argument model.
/// - Validates arity, named-target lookup, duplicate detection, positional-before-named ordering,
///   default filling, missing required-field detection, and per-field type compatibility.
/// - Fills trailing fields from struct defaults when arguments are omitted.
/// - Produces a canonical `Expression::struct_instance` with definition-order fields.
/// - Retains the whole constructor as one pending call when a concrete field argument depends on
///   an unresolved numeric origin, so the body root resumes this same construction exactly once.
///
/// WHY:
/// - Constructor syntax is syntactically identical to function call syntax; sharing the same
///   argument-resolution machinery keeps the two forms consistent and avoids a parallel
///   resolution system.
/// - Argument compatibility cannot run before the arguments' numeric types are selected, and a
///   concrete field type is already resolved, so the constructor keeps its identity instead of
///   bubbling a fragment its statement position cannot finish.
/// - Const-record coercion for top-level compile-time constants is applied after resolution.
pub(super) fn parse_struct_constructor_expression(
    token_stream: &mut AstCursor,
    input: StructConstructorParseInput<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<ParsedConstructorExpression, ExpressionParseError> {
    let StructConstructorParseInput {
        struct_path,
        struct_name,
        fields,
        struct_value_mode,
        type_id,
    } = input;

    let constructor_span = Some(token_stream.current_span());
    let struct_name_display = string_table.resolve(struct_name).to_owned();

    // The stream is positioned on the struct symbol when called.
    // Advance past it to '(' so the shared call-argument parser can take over.
    token_stream.advance();

    if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        return Err(CompilerError::compiler_error(
            "Struct constructor parser called without an opening parenthesis",
        )
        .into());
    }

    // ------------------------
    //  Seed generic context before parsing field values
    // ------------------------
    let constructor_field_views = ConstructorField::from_struct_declarations(fields);
    let is_generic_struct = context
        .generic_declarations_by_path
        .as_ref()
        .and_then(|generic_declarations| generic_declarations.get(struct_path))
        .is_some_and(|kind| matches!(kind, GenericDeclarationKind::Struct));
    let generic_constructor_seed = if is_generic_struct {
        Some(seed_generic_nominal_constructor(
            GenericNominalConstructorContext {
                nominal_path: struct_path,
                display_name: &struct_name_display,
                template: GenericNominalTemplate::StructFields(&constructor_field_views),
                span: constructor_span,
            },
            context,
            type_interner.environment(),
            string_table,
        )?)
    } else {
        None
    };

    let mut field_expectations =
        expectations_from_constructor_fields(&constructor_field_views, path_fork);
    if let Some(seed) = &generic_constructor_seed {
        seed.bind_parse_expectations(
            &mut field_expectations,
            type_interner.environment_mut_for_derived_types(),
        );
    }

    // ------------------------
    //  Parse arguments
    // ------------------------
    // A field whose type is still a generic parameter has no numeric type yet, so its numeric
    // argument stays pending. The constructor then suspends as one operand; its instance is
    // inferred from completed argument types at the body root. Concrete fields retain the same way.
    let parsed_arguments = parse_call_arguments_typed_with_numeric_retention(
        token_stream,
        context,
        type_interner,
        string_table,
        &field_expectations,
        CallArgumentSyntax::Supported {
            callee_name: Some(struct_name),
        },
        path_fork,
    )?;

    let raw_args = match parsed_arguments {
        ParsedCallArguments::Ready(arguments) => arguments,
        // A retained constructor keeps its resolved identity, field views and construction policy
        // with the operand. A generic constructor also links its declared self shape now, so the
        // consumer receives the nominal identity before the numeric origins resolve.
        ParsedCallArguments::Retained(arguments) => {
            let operand = match generic_constructor_seed {
                Some(seed) => {
                    let result_shape =
                        link_generic_nominal_result_shape(GenericNominalShapeInput {
                            nominal_path: struct_path,
                            display_name: &struct_name_display,
                            declared_fields: &constructor_field_views,
                            arguments: &arguments,
                            seeded_bindings: seed.bindings(),
                            context,
                            type_interner,
                            string_table,
                            call_span: constructor_span,
                        })?;
                    DeferredGenericNominalCall {
                        nominal_path: struct_path.to_owned(),
                        display_name: struct_name_display,
                        seed,
                        arguments,
                        result_shape,
                        call_span: constructor_span,
                        constructor: GenericNominalConstructor::Struct {
                            fields: constructor_field_views,
                            struct_value_mode: struct_value_mode.as_owned(),
                            enforce_const_record: context.kind.allows_const_record_coercion(),
                        },
                    }
                    .into_pending_operand()
                }
                None => {
                    let deferred = DeferredStructCall {
                        struct_path: struct_path.to_owned(),
                        struct_name,
                        type_id,
                        fields: constructor_field_views,
                        arguments,
                        call_span: constructor_span,
                        struct_value_mode: struct_value_mode.as_owned(),
                        enforce_const_record: context.kind.allows_const_record_coercion(),
                    };
                    ExpressionRpnItem::PendingNumericCall {
                        call: Box::new(DeferredCall::Struct(Box::new(deferred))),
                        span: constructor_span,
                    }
                }
            };
            return Ok(ParsedConstructorExpression::RetainedOperand(operand));
        }
    };

    // ------------------------
    //  Construct the instance
    // ------------------------
    if let Some(seed) = generic_constructor_seed {
        let struct_expr = complete_generic_struct_instance(
            GenericStructCompletion {
                nominal_path: struct_path,
                display_name: &struct_name_display,
                template_fields: &constructor_field_views,
                struct_value_mode,
                enforce_const_record: context.kind.allows_const_record_coercion(),
                constructor_span,
                seed,
                arguments: &raw_args,
            },
            context,
            type_interner,
            path_fork,
            string_table,
        )?;
        return Ok(ParsedConstructorExpression::Value(struct_expr));
    }

    let struct_expr = finish_struct_instance(
        StructInstanceInput {
            struct_path,
            struct_name_display: &struct_name_display,
            fields: &constructor_field_views,
            constructor_span,
            struct_value_mode,
            enforce_const_record: context.kind.allows_const_record_coercion(),
            generic_instance_key: None,
            instance_type_id: type_id,
        },
        &raw_args,
        context,
        type_interner,
        path_fork,
        string_table,
    )?;

    Ok(ParsedConstructorExpression::Value(struct_expr))
}

/// Resolved constructor facts shared by the direct and retained completion paths.
struct StructInstanceInput<'a> {
    struct_path: &'a PathId,
    struct_name_display: &'a str,
    fields: &'a [ConstructorField],
    constructor_span: Option<SourceSpan>,
    struct_value_mode: &'a ValueMode,
    enforce_const_record: bool,
    generic_instance_key: Option<GenericInstantiationKey>,
    instance_type_id: TypeId,
}

/// Validate one struct constructor's arguments and build the canonical struct instance.
///
/// WHAT: the single construction tail for both the direct path and the retained-call resumption:
///       argument compatibility, const-context field checks, default filling through the shared
///       call validator and definition-order field building.
/// WHY: a retained constructor must resume through the exact ordinary construction rules, so the
///      two lanes share one finisher instead of a second validator that could drift.
fn finish_struct_instance(
    input: StructInstanceInput<'_>,
    arguments: &[CallArgument],
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let StructInstanceInput {
        struct_path,
        struct_name_display,
        fields,
        constructor_span,
        struct_value_mode,
        enforce_const_record,
        generic_instance_key,
        instance_type_id,
    } = input;

    // ------------------------
    //  Validate arguments against fields
    // ------------------------
    let expectations = expectations_from_constructor_fields(fields, path_fork);
    let type_check_context = type_interner.type_check_context();
    let resolved_args = resolve_call_arguments(
        CallDiagnosticContext::struct_constructor(struct_name_display),
        arguments,
        &expectations,
        constructor_span,
        CallArgumentResolutionContext {
            string_table,
            path_fork,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )?;

    // ------------------------
    //  Build fields with const checks
    // ------------------------
    let mut struct_fields = Vec::with_capacity(fields.len());

    for (field, arg) in fields.iter().zip(resolved_args) {
        let mut value = arg.value;

        if enforce_const_record {
            // Header-stage struct shells may carry placeholder references until AST environment
            // construction resolves constants in graph order.
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

            // Const records are data-only exports, so mutable ownership is removed to keep
            // constant semantics explicit in later stages.
            value.value_mode = ValueMode::ImmutableOwned;
        }

        struct_fields.push(Declaration {
            id: field.name,
            value,
            binding_span: None,
            config_qualifier: None,
        });
    }

    // Const records force immutable ownership; otherwise inherit the struct's declared mode.
    let instance_ownership = if enforce_const_record {
        ValueMode::ImmutableOwned
    } else {
        struct_value_mode.as_owned()
    };

    Ok(Expression::struct_instance(
        struct_path.to_owned(),
        struct_fields,
        constructor_span,
        instance_ownership,
        enforce_const_record,
        generic_instance_key,
        instance_type_id,
    ))
}

/// Generic struct constructor facts shared by the immediate and retained completion paths.
///
/// WHAT: the nominal identity, the declared field views the instance is built from, the
///       construction policy, the contextual seed, and the completed argument list.
/// WHY: a generic instance is inferred from completed argument types, so both lanes must reach the
///      same inference and finisher with the same facts.
pub(crate) struct GenericStructCompletion<'a> {
    pub(crate) nominal_path: &'a PathId,
    pub(crate) display_name: &'a str,
    pub(crate) template_fields: &'a [ConstructorField],
    pub(crate) struct_value_mode: &'a ValueMode,
    pub(crate) enforce_const_record: bool,
    pub(crate) constructor_span: Option<SourceSpan>,
    pub(crate) seed: GenericNominalConstructorSeed,
    pub(crate) arguments: &'a [CallArgument],
}

/// Infer one generic struct instance from completed arguments and build its value.
///
/// WHAT: infers the instance from the seed and the argument types, substitutes the declared fields
///       with the inferred arguments, then runs the shared struct finisher.
/// WHY: the instance is selected only after every argument is concrete, so the immediate and
///      retained lanes both end at this one completion.
pub(crate) fn complete_generic_struct_instance(
    completion: GenericStructCompletion<'_>,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    path_fork: &mut PathInternerFork,
    string_table: &mut StringTable,
) -> Result<Expression, ExpressionParseError> {
    let GenericStructCompletion {
        nominal_path,
        display_name,
        template_fields,
        struct_value_mode,
        enforce_const_record,
        constructor_span,
        seed,
        arguments,
    } = completion;

    let inference = infer_generic_nominal_constructor(
        GenericNominalConstructorInput {
            context: GenericNominalConstructorContext {
                nominal_path,
                display_name,
                template: GenericNominalTemplate::StructFields(template_fields),
                span: constructor_span,
            },
            constructor_fields: Some(template_fields),
            raw_args: Some(arguments),
            path_fork,
        },
        seed,
        context,
        type_interner,
        string_table,
    )?;

    // Substitute the declared fields with the inferred arguments, as the choice lane does. The
    // instance's cached view is not read, since an early header-stage completion can run before the
    // base nominal's views are populated. Defaults stay with the declared field views.
    let resolved_fields: Vec<ConstructorField> = {
        let replacements = inference.bindings.replacement_map();
        let type_env = type_interner.environment_mut_for_derived_types();
        template_fields
            .iter()
            .map(|field| ConstructorField {
                type_id: type_env.substitute_type_id(field.type_id, replacements),
                ..field.clone()
            })
            .collect()
    };

    finish_struct_instance(
        StructInstanceInput {
            struct_path: nominal_path,
            struct_name_display: display_name,
            fields: &resolved_fields,
            constructor_span,
            struct_value_mode,
            enforce_const_record,
            generic_instance_key: inference.instance_key,
            instance_type_id: inference.instance_type_id,
        },
        arguments,
        context,
        type_interner,
        path_fork,
        string_table,
    )
}

/// One concrete struct constructor retained until its numeric-dependent arguments resolve.
///
/// WHAT: the resolved struct identity and canonical result type, the field views (including the
///       authored defaults the semantic environment does not carry), the ordered argument list
///       with its authored routing, and the construction policy captured at the call site.
/// WHY: the field types are already concrete, so argument compatibility cannot run before the
///      arguments' numeric origins are selected. Keeping the resolved identity and views means
///      the body root resumes the ordinary constructor exactly once: no second argument parse,
///      no struct-declaration re-lookup and no fabricated `TypeId`.
#[derive(Debug, Clone)]
pub(crate) struct DeferredStructCall {
    pub(crate) struct_path: PathId,
    pub(crate) struct_name: StringId,
    pub(crate) type_id: TypeId,
    pub(crate) fields: Vec<ConstructorField>,
    pub(crate) arguments: Vec<DeferredCallArgument>,
    pub(crate) call_span: Option<SourceSpan>,
    pub(crate) struct_value_mode: ValueMode,
    pub(crate) enforce_const_record: bool,
}

/// Resume one retained concrete struct constructor after its arguments resolve.
///
/// WHAT: completes every retained argument through the session's shared call-argument service,
///       then validates and builds the canonical struct instance through the same finisher the
///       direct path uses.
/// WHY: the constructor was suspended before argument compatibility, so its own receiving
///      boundary is the only place the completed argument types may be checked and the
///      definition-order fields (with defaults) may be produced.
pub(crate) fn complete_deferred_struct_call<
    'env,
    S: DeferredExpressionCompletion<'env> + ?Sized,
>(
    deferred: DeferredStructCall,
    session: &mut S,
) -> Result<Expression, ExpressionParseError> {
    let DeferredStructCall {
        struct_path,
        struct_name,
        type_id,
        fields,
        arguments,
        call_span,
        struct_value_mode,
        enforce_const_record,
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
    let struct_name_display = string_table.resolve(struct_name).to_owned();
    finish_struct_instance(
        StructInstanceInput {
            struct_path: &struct_path,
            struct_name_display: &struct_name_display,
            fields: &fields,
            constructor_span: call_span,
            struct_value_mode: &struct_value_mode,
            enforce_const_record,
            generic_instance_key: None,
            instance_type_id: type_id,
        },
        &completed_arguments,
        context,
        type_interner,
        path_fork,
        string_table,
    )
}
