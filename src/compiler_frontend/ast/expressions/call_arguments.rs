//! Shared call-argument syntax and retained parameter-slot routing.
//!
//! WHAT: owns parentheses, separators, named targets, mutable markers, expression boundaries,
//!       cast-target threading and the one parser-time named/positional slot router used by every
//!       call-shaped AST surface. Typed receiving policies select naming and value requirements
//!       without changing that list ownership.
//! WHY: each call consumer must parse the same syntax once and carry the selected parameter slot
//!      into final validation instead of rebuilding call meaning after expression parsing.
//!
//! This module does not own call result handling, defaults, type compatibility, access
//! validation, generic inference or call-specific AST construction. Those policies consume the
//! [`crate::compiler_frontend::ast::expressions::call_argument::ParameterSlot`] retained here.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    ExpectedParameterType, ParameterExpectation,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticToken,
    InvalidBuiltinCallReason, InvalidCallShapeReason, InvalidGenericInstantiationReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Naming policy selected by a receiving call-shaped surface.
///
/// WHAT: keeps declaration-order signature routing distinct from named-only field routing.
/// WHY: named-only values have no declaration slots to fabricate, while ordinary calls retain
/// their existing positional-then-named semantics.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallArgumentNamingPolicy {
    PositionalThenNamed,
    PositionalOnly,
    NamedOnly,
}

/// Value-evaluation policy selected by a receiving call-shaped surface.
///
/// WHAT: records whether the receiving surface accepts ordinary expressions or requires a
/// compile-time value.
/// WHY: the shared owner must carry this distinction without adding a second expression parser.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallArgumentValuePolicy {
    Ordinary,
    ConstRequired,
}

/// Surface diagnostic descriptor for one call-shaped owner.
///
/// WHAT: carries the existing call/builtin diagnostic lane and no-argument builtin policy.
/// WHY: naming and value evaluation are typed receiving policies, while this descriptor preserves
/// current diagnostics and call-site compatibility.
#[derive(Clone, Copy)]
pub(crate) enum CallArgumentSyntax {
    Supported {
        callee_name: Option<StringId>,
    },
    UnsupportedCall {
        callee_name: Option<StringId>,
    },
    UnsupportedBuiltinMember {
        member_name: Option<StringId>,
        takes_no_arguments: bool,
    },
}

/// Diagnostic identity carried by the shared argument owner.
///
/// WHAT: keeps the existing call/builtin diagnostic lane together with the optional operation
/// name used by a const-required receiving context.
/// WHY: naming and value policies must not manufacture a second parser or lose the receiving
/// surface that owns source diagnostics.
#[derive(Clone, Copy)]
pub(crate) struct CallArgumentDiagnosticContext {
    pub(crate) syntax: CallArgumentSyntax,
    pub(crate) const_operation: Option<StringId>,
    /// Opening-delimiter span retained for receiving-boundary diagnostics.
    ///
    /// WHAT: preserves the authored `(` span independently from each entry span.
    /// WHY: empty, nested and const-required boundary diagnostics may need the receiving
    /// delimiter even when no value expression exists.
    pub(crate) opening_span: Option<SourceSpan>,
}

impl CallArgumentDiagnosticContext {
    pub(crate) fn from_syntax(syntax: CallArgumentSyntax) -> Self {
        let const_operation = match syntax {
            CallArgumentSyntax::Supported { callee_name }
            | CallArgumentSyntax::UnsupportedCall { callee_name } => callee_name,
            CallArgumentSyntax::UnsupportedBuiltinMember { member_name, .. } => member_name,
        };

        Self {
            syntax,
            const_operation,
            opening_span: None,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_const_operation(mut self, const_operation: Option<StringId>) -> Self {
        self.const_operation = const_operation;
        self
    }

    pub(crate) fn with_opening_span(mut self, opening_span: Option<SourceSpan>) -> Self {
        self.opening_span = opening_span;
        self
    }
}

/// Typed receiving context for one shared call-argument list.
///
/// WHAT: carries naming, value-evaluation and diagnostic policy through the one parenthesised
/// list loop.
/// WHY: future named-only/const-required surfaces can select their policy without fabricating
/// declaration slots or introducing a second delimiter parser.
#[derive(Clone, Copy)]
pub(crate) struct CallArgumentReceivingContext {
    pub(crate) naming_policy: CallArgumentNamingPolicy,
    pub(crate) value_policy: CallArgumentValuePolicy,
    pub(crate) diagnostics: CallArgumentDiagnosticContext,
}

impl CallArgumentReceivingContext {
    pub(crate) fn existing_call(syntax: CallArgumentSyntax) -> Self {
        let naming_policy = match syntax {
            CallArgumentSyntax::Supported { .. } => CallArgumentNamingPolicy::PositionalThenNamed,
            CallArgumentSyntax::UnsupportedCall { .. }
            | CallArgumentSyntax::UnsupportedBuiltinMember { .. } => {
                CallArgumentNamingPolicy::PositionalOnly
            }
        };

        Self {
            naming_policy,
            value_policy: CallArgumentValuePolicy::Ordinary,
            diagnostics: CallArgumentDiagnosticContext::from_syntax(syntax),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn with_policies(
        diagnostics: CallArgumentDiagnosticContext,
        naming_policy: CallArgumentNamingPolicy,
        value_policy: CallArgumentValuePolicy,
    ) -> Self {
        Self {
            naming_policy,
            value_policy,
            diagnostics,
        }
    }
}

/// Parses one call-shaped list through the shared receiving-context policy.
///
/// The `expectations` slice is only a declaration-order signature. Named-only receivers pass
/// `None` (or have it ignored by this owner), so no synthetic parameter slots are created.
#[allow(
    clippy::too_many_arguments,
    reason = "argument parsing keeps the token stream, scope, mutable interner/string/path state, syntax contexts, and optional expectations as separate borrows"
)]
pub(crate) fn parse_call_arguments_with_receiving_context(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    receiving_context: CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
    syntax_context: CallArgumentSyntaxContext,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        string_table,
        syntax_context,
        receiving_context,
        expectations,
        path_fork,
    )
}

/// Parses a call argument list with explicit parameter expectations threaded into each argument.
///
/// WHAT: gives every argument expression a `CastTargetContext` derived from its corresponding
///      parameter type, so `cast` / `cast!` can resolve at concrete source/receiver/host
///      parameters and generic parameter slots can reject `cast` with `TargetIsGenericParameter`.
/// WHY: raw call parsing used to resolve arguments before validation. Threading expectations keeps
///      the cast-target channel narrow and local to the argument parser.
pub(crate) fn parse_call_arguments_typed_with_expectations(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    expectations: &[ParameterExpectation],
    argument_syntax: CallArgumentSyntax,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        string_table,
        CallArgumentReceivingContext::existing_call(argument_syntax),
        Some(expectations),
        CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )
}

pub(crate) fn parse_generic_call_arguments_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    generic_function_name: Option<StringId>,
    expectations: &[ParameterExpectation],
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        string_table,
        CallArgumentReceivingContext::existing_call(CallArgumentSyntax::Supported {
            callee_name: generic_function_name,
        }),
        Some(expectations),
        CallArgumentSyntaxContext::GenericFunction {
            function_name: generic_function_name,
        },
        path_fork,
    )
}

#[derive(Clone, Copy)]
pub(crate) enum CallArgumentSyntaxContext {
    Ordinary,
    GenericFunction { function_name: Option<StringId> },
}

/// Preserves parse-time context for literals whose type cannot be inferred from their token.
///
/// WHAT: passes an optional parameter type into expression parsing so a call argument such as
///      `message = none` can resolve its inner type before ordinary call validation runs.
/// WHY: call arguments otherwise parse with natural-type inference, which is correct for most
///      values but rejects context-sensitive `none` literals before the receiving slot is known.
///      Collection and map targets remain inferred here so their type diagnostics stay owned by
///      call validation rather than moving into the generic expression parser.
fn expected_type_for_parameter_expectation(
    expectation: &ParameterExpectation,
    type_environment: &TypeEnvironment,
) -> ExpectedType {
    match expectation.expected_type {
        ExpectedParameterType::Known(type_id) if type_environment.is_option(type_id) => {
            parse_expectation_for_type_id(type_id, type_environment)
        }
        ExpectedParameterType::Known(_) | ExpectedParameterType::UnknownExternal => {
            ExpectedType::Infer
        }
    }
}

/// Builds a `CastTargetContext` from a single parameter expectation.
///
/// WHAT: converts the parameter's expected type into the same cast-target channel used by
///      declarations and assignments. `UnknownExternal` parameters are not builtin cast targets.
/// WHY: keeps call arguments consistent with other explicit typed boundaries without making
///      ordinary expression parsing globally type-directed.
fn cast_target_context_for_parameter_expectation(
    expectation: &ParameterExpectation,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> CastTargetContext {
    match expectation.expected_type {
        ExpectedParameterType::Known(type_id) => {
            cast_target_context_for_type_id(type_id, type_environment, string_table, path_fork)
        }
        ExpectedParameterType::UnknownExternal => CastTargetContext::None,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "argument parsing keeps the token stream, scope, mutable interner/string/path state, syntax contexts, and optional expectations as separate borrows"
)]
fn parse_call_arguments_inner(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    syntax_context: CallArgumentSyntaxContext,
    receiving_context: CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    ast_log!("Creating function call arguments");
    let mut receiving_context = receiving_context;
    let opening_span = current_span(token_stream);
    if receiving_context.diagnostics.opening_span.is_none() {
        receiving_context.diagnostics = receiving_context
            .diagnostics
            .with_opening_span(opening_span);
    }

    let argument_syntax = receiving_context.diagnostics.syntax;
    // Named-only receivers own fields by label, not by declaration-order signature slots.
    // Keeping this as `None` is the invariant that prevents fabricated `ParameterSlot`s.
    let routed_expectations = match receiving_context.naming_policy {
        CallArgumentNamingPolicy::NamedOnly => None,
        CallArgumentNamingPolicy::PositionalThenNamed
        | CallArgumentNamingPolicy::PositionalOnly => expectations,
    };

    if let CallArgumentSyntax::UnsupportedBuiltinMember {
        member_name: Some(member_name),
        takes_no_arguments: true,
    } = argument_syntax
    {
        if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::MissingParentheses,
                Some(member_name),
                current_span(token_stream),
            )
            .into());
        }

        token_stream.advance();
        token_stream.skip_newlines();

        if token_stream.current_tag() != TokenTag::CLOSE_PARENTHESIS {
            return Err(CompilerDiagnostic::invalid_builtin_call(
                InvalidBuiltinCallReason::TakesNoArguments,
                Some(member_name),
                current_span(token_stream),
            )
            .into());
        }

        token_stream.advance();
        return Ok(Vec::new());
    }

    // ------------------------
    //  Consume opening paren
    // ------------------------
    if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        let found = token_stream
            .current_diagnostic_token(string_table)
            .map_err(|error| {
                CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "call opening-delimiter diagnostic",
                )
            })?;
        return Err(CompilerDiagnostic::expected_token_from_tags(
            TokenTag::OPEN_PARENTHESIS,
            found,
            current_span(token_stream),
        )
        .into());
    }

    token_stream.advance();
    token_stream.skip_newlines();

    if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
        token_stream.advance();
        return Ok(Vec::new());
    }

    let mut arguments = Vec::new();
    let mut slot_router = ParameterSlotRouter::new(routed_expectations, receiving_context);
    // ------------------------
    //  Parse each argument
    // ------------------------
    loop {
        token_stream.skip_newlines();
        if token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS {
            token_stream.advance();
            break;
        }

        let argument_span = current_span(token_stream);

        reject_simple_generic_argument_type_ascription(token_stream, syntax_context)?;
        let lookahead_base = token_stream.position();

        // Detect named-target syntax (`name = expr`) or reject unsupported variants.
        // Pure lookahead only: probe deeper tokens without advancing the stream.
        let lookahead_third_is_assign =
            lookahead_tag_at(token_stream, lookahead_base.saturating_add(2))
                == Some(TokenTag::ASSIGN);
        let lookahead_third_is_close_paren =
            lookahead_tag_at(token_stream, lookahead_base.saturating_add(2))
                == Some(TokenTag::CLOSE_PARENTHESIS);
        let lookahead_fourth_is_assign =
            lookahead_tag_at(token_stream, lookahead_base.saturating_add(3))
                == Some(TokenTag::ASSIGN);
        let named_target = match token_stream.current_tag() {
            // `~name = expr` is not supported.
            TokenTag::MUTABLE
                if next_token_tag(token_stream) == Some(TokenTag::SYMBOL)
                    && lookahead_third_is_assign =>
            {
                return Err(CompilerDiagnostic::unexpected_token_from_tag(
                    DiagnosticToken::from_static_tag(TokenTag::MUTABLE),
                    current_span(token_stream),
                )
                .into());
            }

            // Standard named argument: `name = expr`.
            TokenTag::SYMBOL if next_token_tag(token_stream) == Some(TokenTag::ASSIGN) => {
                let target_span = current_span(token_stream);
                let target_name = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                            "named argument symbol had no string payload",
                        )
                    })?;
                token_stream.advance();
                token_stream.advance();
                token_stream.skip_newlines();
                Some((target_name, target_span))
            }

            // Parenthesized names like `(name) = expr` are not supported.
            TokenTag::OPEN_PARENTHESIS
                if next_token_tag(token_stream) == Some(TokenTag::SYMBOL)
                    && lookahead_third_is_close_paren
                    && lookahead_fourth_is_assign =>
            {
                return Err(CompilerDiagnostic::unexpected_token_from_tag(
                    DiagnosticToken::from_static_tag(TokenTag::OPEN_PARENTHESIS),
                    current_span(token_stream),
                )
                .into());
            }

            _ => None,
        };

        if matches!(
            receiving_context.naming_policy,
            CallArgumentNamingPolicy::NamedOnly
        ) && named_target.is_none()
        {
            // A named-only receiver has no signature-slot diagnostic to report here. Keep the
            // parser-level ownership truthful by rejecting the positional token directly; Phase
            // 2 field validation owns duplicate/unknown-label diagnostics.
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "named-only argument diagnostic",
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            return Err(CompilerDiagnostic::unexpected_token_from_tag(found, argument_span).into());
        }

        let parameter_slot = slot_router.route(named_target.as_ref(), argument_span)?;

        let (access_mode, marker_span) = if token_stream.current_tag() == TokenTag::MUTABLE {
            let marker_span = current_span(token_stream);
            token_stream.advance();
            (CallAccessMode::Mutable, marker_span)
        } else {
            (CallAccessMode::Shared, None)
        };

        // A named target or access mode without a following value is an error.
        if token_stream.current_tag() == TokenTag::COMMA
            || token_stream.current_tag() == TokenTag::CLOSE_PARENTHESIS
        {
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "call argument value diagnostic",
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            return Err(CompilerDiagnostic::unexpected_token_from_tag(
                found,
                current_span(token_stream),
            )
            .into());
        }

        let parameter_expectation = parameter_slot
            .and_then(|slot| routed_expectations.and_then(|items| items.get(slot.index())));
        // Only a bare `none` needs the receiving option slot during parsing. Ordinary values
        // retain natural inference so call validation owns their type diagnostics.
        let mut inferred = if argument_is_bare_none(token_stream) {
            parameter_expectation
                .map(|expectation| {
                    expected_type_for_parameter_expectation(
                        expectation,
                        type_interner.environment(),
                    )
                })
                .unwrap_or(ExpectedType::Infer)
        } else {
            ExpectedType::Infer
        };
        let cast_target_context = parameter_expectation
            .map(|expectation| {
                cast_target_context_for_parameter_expectation(
                    expectation,
                    type_interner.environment(),
                    string_table,
                    &*path_fork,
                )
            })
            .unwrap_or(CastTargetContext::None);
        let mut cast_target_context = cast_target_context;
        let input = ExpressionParseInput::without_boundary_catch(
            ExpressionParseResources {
                token_stream,
                scope_context: context,
                type_interner,
                expected_type: &mut inferred,
                cast_target_context: &mut cast_target_context,
                value_mode: &ValueMode::ImmutableOwned,
                string_table,
                path_fork,
            },
            false,
        );
        let value = create_expression_with_trailing_newline_policy(input)?;
        validate_argument_value_policy(&value, context, path_fork, receiving_context)?;

        // Preserve the value-expression span, named-parameter span, and authored `~` marker span
        // independently so diagnostics can point at whichever source the author must change.
        let value_span = value.span;
        let argument = if let Some((name, target_span)) = named_target {
            CallArgument::named(value, name, access_mode, value_span, target_span)
        } else {
            CallArgument::positional(value, access_mode, value_span)
        };
        let argument = argument.with_marker_span(marker_span);
        let argument = if let Some(parameter_slot) = parameter_slot {
            argument.with_parameter_slot(parameter_slot)
        } else {
            argument
        };

        let (argument, list_ended) = match token_stream.current_tag() {
            TokenTag::COMMA => {
                let separator_span = current_span(token_stream);
                token_stream.advance();
                token_stream.skip_newlines();
                (
                    argument.with_following_separator_span(separator_span),
                    false,
                )
            }
            TokenTag::CLOSE_PARENTHESIS => {
                token_stream.advance();
                (argument, true)
            }
            _ => {
                let found = token_stream
                    .current_diagnostic_token(string_table)
                    .map_err(|error| {
                        CompilerDiagnostic::token_view_invariant_error(
                            error,
                            "call argument separator diagnostic",
                        )
                    })?
                    .unwrap_or_else(|| {
                        DiagnosticToken::from_static_tag(token_stream.current_tag())
                    });
                return Err(CompilerDiagnostic::unexpected_token_from_tag(
                    found,
                    current_span(token_stream),
                )
                .into());
            }
        };

        arguments.push(argument);
        if list_ended {
            break;
        }
    }

    Ok(arguments)
}

/// Returns whether the current argument is exactly `none` before its call delimiter.
///
fn argument_is_bare_none(token_stream: &AstCursor) -> bool {
    if token_stream.current_tag() != TokenTag::NONE_LITERAL {
        return false;
    }

    let mut next_index = token_stream.position().saturating_add(1);
    while lookahead_tag_at(token_stream, next_index) == Some(TokenTag::NEWLINE) {
        next_index = next_index.saturating_add(1);
    }

    lookahead_tag_at(token_stream, next_index)
        .is_some_and(|tag| matches!(tag, TokenTag::COMMA | TokenTag::CLOSE_PARENTHESIS))
}
/// Apply a receiving value policy after ordinary expression parsing has established the value.
///
/// WHAT: const-required receivers reuse the canonical expression const classifier and module-local
/// template store instead of token filtering or a second evaluator.
/// WHY: expression parsing still owns grouping, casts, generic inference and contextual `none`;
/// this check only decides whether the completed value satisfies the receiving boundary.
fn validate_argument_value_policy(
    value: &Expression,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
    receiving_context: CallArgumentReceivingContext,
) -> Result<(), ExpressionParseError> {
    if !matches!(
        receiving_context.value_policy,
        CallArgumentValuePolicy::ConstRequired
    ) {
        return Ok(());
    }

    let is_placeholder_reference = if let ExpressionKind::Reference(path) = &value.kind {
        path_fork.component(*path).is_some_and(|name| {
            context.get_reference(&name).is_some_and(|reference| {
                reference
                    .as_declaration()
                    .is_unresolved_constant_placeholder()
            })
        })
    } else {
        false
    };

    let is_compile_time_value = is_placeholder_reference
        || value
            .const_value_kind_with_template_classifier(&mut |template| {
                classify_template_from_effective_tir(template, &context.template_ir_store)
            })?
            .is_compile_time_value();

    if is_compile_time_value {
        return Ok(());
    }

    Err(CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
        receiving_context.diagnostics.const_operation,
        value.span,
    )
    .into())
}

struct ParameterSlotRouter<'a> {
    expectations: Option<&'a [ParameterExpectation]>,
    receiving_context: CallArgumentReceivingContext,
    parameter_name_to_slot: FxHashMap<StringId, usize>,
    positional_cursor: usize,
    saw_named_argument: bool,
    occupied_parameter_slots: Option<Vec<bool>>,
}

impl<'a> ParameterSlotRouter<'a> {
    fn new(
        expectations: Option<&'a [ParameterExpectation]>,
        receiving_context: CallArgumentReceivingContext,
    ) -> Self {
        let parameter_name_to_slot = expectations
            .map(|items| {
                items
                    .iter()
                    .enumerate()
                    .filter_map(|(index, expectation)| expectation.name.map(|name| (name, index)))
                    .collect()
            })
            .unwrap_or_default();

        Self {
            expectations,
            receiving_context,
            parameter_name_to_slot,
            positional_cursor: 0,
            saw_named_argument: false,
            occupied_parameter_slots: expectations.map(|items| vec![false; items.len()]),
        }
    }

    fn route(
        &mut self,
        named_target: Option<&(StringId, Option<SourceSpan>)>,
        argument_span: Option<SourceSpan>,
    ) -> Result<Option<ParameterSlot>, ExpressionParseError> {
        let Some(expectations) = self.expectations else {
            if named_target.is_some() {
                self.saw_named_argument = true;
            } else {
                self.positional_cursor += 1;
            }

            return Ok(None);
        };

        if let Some((target_name, target_span)) = named_target {
            self.saw_named_argument = true;

            if matches!(
                self.receiving_context.naming_policy,
                CallArgumentNamingPolicy::PositionalOnly
            ) && matches!(
                self.receiving_context.diagnostics.syntax,
                CallArgumentSyntax::Supported { .. }
            ) {
                return Err(CompilerDiagnostic::invalid_call_shape(
                    InvalidCallShapeReason::NamedArgumentsNotSupported,
                    self.callee_name(),
                    *target_span,
                )
                .into());
            }

            match self.receiving_context.diagnostics.syntax {
                CallArgumentSyntax::UnsupportedCall { callee_name } => {
                    return Err(CompilerDiagnostic::invalid_call_shape(
                        InvalidCallShapeReason::NamedArgumentsNotSupported,
                        callee_name,
                        *target_span,
                    )
                    .into());
                }

                CallArgumentSyntax::UnsupportedBuiltinMember { member_name, .. } => {
                    return Err(CompilerDiagnostic::invalid_builtin_call(
                        InvalidBuiltinCallReason::NamedArgumentsNotSupported,
                        member_name,
                        *target_span,
                    )
                    .into());
                }

                CallArgumentSyntax::Supported { callee_name } => {
                    let Some(slot) = self.parameter_name_to_slot.get(target_name).copied() else {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::NamedArgumentNotFound {
                                name: *target_name,
                                known_parameters: known_parameter_names(expectations),
                            },
                            callee_name,
                            *target_span,
                        )
                        .into());
                    };

                    self.mark_slot_occupied(slot, *target_span)?;
                    return Ok(Some(ParameterSlot::new(slot)));
                }
            }
        }

        if self.saw_named_argument {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::PositionalAfterNamed,
                self.callee_name(),
                argument_span,
            )
            .into());
        }

        let callee_name = self.callee_name();
        let Some(occupied_slots) = &mut self.occupied_parameter_slots else {
            let slot = self.positional_cursor;
            self.positional_cursor += 1;
            return Ok(Some(ParameterSlot::new(slot)));
        };

        while self.positional_cursor < occupied_slots.len()
            && occupied_slots[self.positional_cursor]
        {
            self.positional_cursor += 1;
        }

        if self.positional_cursor >= occupied_slots.len() {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::ExtraPositionalArgument {
                    expected_count: expectations.len(),
                },
                callee_name,
                argument_span,
            )
            .into());
        }

        let slot = self.positional_cursor;
        occupied_slots[slot] = true;
        self.positional_cursor += 1;
        Ok(Some(ParameterSlot::new(slot)))
    }

    fn mark_slot_occupied(
        &mut self,
        slot: usize,
        span: Option<SourceSpan>,
    ) -> Result<(), ExpressionParseError> {
        let parameter_name = self
            .expectations
            .and_then(|items| items.get(slot))
            .and_then(|expectation| expectation.name);
        let callee_name = self.callee_name();
        let Some(occupied_slots) = &mut self.occupied_parameter_slots else {
            return Ok(());
        };

        if occupied_slots[slot] {
            let diagnostic = CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::DuplicateArgument {
                    parameter_name,
                    parameter_index: slot,
                },
                callee_name,
                span,
            );
            return Err(diagnostic.into());
        }

        occupied_slots[slot] = true;
        Ok(())
    }

    fn callee_name(&self) -> Option<StringId> {
        match self.receiving_context.diagnostics.syntax {
            CallArgumentSyntax::Supported { callee_name }
            | CallArgumentSyntax::UnsupportedCall { callee_name } => callee_name,
            CallArgumentSyntax::UnsupportedBuiltinMember { .. } => None,
        }
    }
}

fn known_parameter_names(expectations: &[ParameterExpectation]) -> Vec<StringId> {
    expectations
        .iter()
        .filter_map(|expectation| expectation.name)
        .collect()
}
/// Read-only tag lookahead through the canonical cursor view.
///
/// WHAT: probes one stable token tag without advancing the stream.
/// WHY: call-argument named-target and bare-`none` scans must not clone wide payload-bearing
/// token values.
fn lookahead_tag_at(token_stream: &AstCursor, index: usize) -> Option<TokenTag> {
    token_stream.token_ref_at(index).map(|token| token.tag())
}

fn next_token_tag(token_stream: &AstCursor) -> Option<TokenTag> {
    token_stream.token_ref_at_offset(1).map(|token| token.tag())
}

fn reject_simple_generic_argument_type_ascription(
    token_stream: &AstCursor,
    syntax_context: CallArgumentSyntaxContext,
) -> Result<(), ExpressionParseError> {
    let CallArgumentSyntaxContext::GenericFunction { function_name } = syntax_context else {
        return Ok(());
    };

    if !starts_simple_value_with_attached_type(token_stream) {
        return Ok(());
    }

    let type_span = token_stream.span_at(token_stream.position().saturating_add(1));
    let Some(type_span) = type_span else {
        return Ok(());
    };

    Err(CompilerDiagnostic::invalid_generic_instantiation(
        function_name,
        InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported,
        Some(type_span),
    )
    .into())
}
/// tries to parse the type keyword as another expression.
///
/// This deliberately stays small: broader type-looking symbol recovery would be speculative in
/// the shared call parser and could change ordinary call errors.
fn starts_simple_value_with_attached_type(token_stream: &AstCursor) -> bool {
    let base = token_stream.position();
    let Some(value_tag) = lookahead_tag_at(token_stream, base) else {
        return false;
    };
    let Some(type_tag) = lookahead_tag_at(token_stream, base.saturating_add(1)) else {
        return false;
    };
    let Some(boundary_tag) = lookahead_tag_at(token_stream, base.saturating_add(2)) else {
        return false;
    };

    matches!(
        value_tag,
        TokenTag::NUMERIC_LITERAL
            | TokenTag::STRING_SLICE_LITERAL
            | TokenTag::BOOL_LITERAL
            | TokenTag::CHAR_LITERAL
            | TokenTag::NONE_LITERAL
    ) && matches!(
        type_tag,
        TokenTag::DATATYPE_INT
            | TokenTag::DATATYPE_FLOAT
            | TokenTag::DATATYPE_BOOL
            | TokenTag::DATATYPE_STRING
            | TokenTag::DATATYPE_CHAR
            | TokenTag::DATATYPE_NONE
    ) && matches!(
        boundary_tag,
        TokenTag::COMMA | TokenTag::CLOSE_PARENTHESIS | TokenTag::NEWLINE
    )
}

#[cfg(test)]
#[path = "tests/function_call_tests.rs"]
mod function_call_tests;
fn current_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    // Call-argument spans are token-local facts on the canonical cursor view.
    Some(token_stream.current_span())
}
