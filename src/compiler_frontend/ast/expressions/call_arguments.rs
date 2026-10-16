//! Shared call-argument syntax and retained parameter-slot routing.
//!
//! WHAT: owns parentheses, separators, named targets, mutable markers, expression boundaries,
//!       cast-target threading, the one parser-time named/positional slot router used by every
//!       call-shaped AST surface, and per-argument numeric-deferral retention for the receiving
//!       lanes that resume a retained call at the body root.
//! WHY: each call consumer must parse the same syntax once and carry the selected parameter slot
//!      into final validation instead of rebuilding call meaning after expression parsing. A
//!      call whose argument needs body-root numeric completion retains that argument's authored
//!      fragment, routing shape and receiving constraint instead of propagating the deferral past
//!      the call that must resume it.
//!
//! This module does not own call result handling, defaults, type compatibility, access
//! validation, generic inference or call-specific AST construction. Those policies consume the
//! [`crate::compiler_frontend::ast::expressions::call_argument::ParameterSlot`] retained here.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, ParameterSlot,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallDiagnosticName, ExpectedParameterType, ParameterExpectation,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_with_trailing_newline_policy, fragment_typed_error_summary,
    register_direct_receiver_witness,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::field_access::reference_expression_from_declaration;
use crate::compiler_frontend::ast::generic_functions::numeric_origins::set_forward_destinations;
use crate::compiler_frontend::ast::module_ast::scope_context::{
    RouteParameterReference, ScopeDeclarationRef,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredCallArgument, DeferredCallArgumentShape, deferred_numeric_expression,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    fragment_shape, register_shape_witness, union_shape_classes,
};
use crate::compiler_frontend::ast::statements::functions::ParameterContract;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticToken,
    InvalidBuiltinCallReason, InvalidCallShapeReason, InvalidExpressionReason,
    InvalidGenericInstantiationReason, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedType, cast_target_context_for_type_id,
    is_numeric_literal_destination_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::FxHashMap;

/// Naming policy selected by a receiving call-shaped surface.
///
/// WHAT: keeps declaration-order signature routing distinct from named-only field routing.
/// WHY: named-only values have no declaration slots to fabricate, while ordinary calls retain
/// their existing positional-then-named semantics.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallArgumentValuePolicy {
    Ordinary,
    ConstRequired,
}

/// Classification of an expression that starts with an opening parenthesis.
///
/// WHAT: inspects only the first significant token and its immediate successor.
/// WHY: anonymous records and ordinary grouping share delimiters; nested calls and later
/// separators belong to their own parsers and must not affect this local decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParenthesizedExpressionKind {
    Empty,
    NamedEntries,
    Group,
}

/// Classify a parenthesized expression without advancing the cursor.
///
/// The caller must leave the cursor on `(`. Newlines immediately after the opener are trivia;
/// a newline between a label and `=` is not part of the named-entry recognizer.
pub(crate) fn classify_parenthesized_expression(
    token_stream: &AstCursor,
) -> ParenthesizedExpressionKind {
    if token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        return ParenthesizedExpressionKind::Group;
    }

    let mut first_index = token_stream.position().saturating_add(1);
    while lookahead_tag_at(token_stream, first_index) == Some(TokenTag::NEWLINE) {
        first_index = first_index.saturating_add(1);
    }

    match lookahead_tag_at(token_stream, first_index) {
        Some(TokenTag::CLOSE_PARENTHESIS) => ParenthesizedExpressionKind::Empty,
        Some(TokenTag::SYMBOL)
            if lookahead_tag_at(token_stream, first_index.saturating_add(1))
                == Some(TokenTag::ASSIGN) =>
        {
            ParenthesizedExpressionKind::NamedEntries
        }
        _ => ParenthesizedExpressionKind::Group,
    }
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
    Constructor {
        callee_name: CallDiagnosticName<'static>,
    },
    Assertion {
        callee_name: Option<StringId>,
    },
    UnsupportedCall {
        callee_name: Option<StringId>,
    },
    UnsupportedBuiltinMember {
        member_name: Option<StringId>,
        takes_no_arguments: bool,
    },
    /// Anonymous const-record fields have no callee or declaration-order slots.
    AnonymousConstRecord,
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
    pub(crate) const_operation: Option<CallDiagnosticName<'static>>,
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
            | CallArgumentSyntax::Assertion { callee_name }
            | CallArgumentSyntax::UnsupportedCall { callee_name } => {
                callee_name.map(CallDiagnosticName::Interned)
            }
            CallArgumentSyntax::Constructor { callee_name } => Some(callee_name),
            CallArgumentSyntax::UnsupportedBuiltinMember { member_name, .. } => {
                member_name.map(CallDiagnosticName::Interned)
            }
            CallArgumentSyntax::AnonymousConstRecord => None,
        };

        Self {
            syntax,
            const_operation,
            opening_span: None,
        }
    }

    pub(crate) fn with_opening_span(mut self, opening_span: Option<SourceSpan>) -> Self {
        self.opening_span = opening_span;
        self
    }
}

/// Whether a receiving owner can retain a numeric-dependent argument for body-root completion.
///
/// WHAT: keeps the per-argument retention choice with the shared list's receiving policy.
/// WHY: owners with a completion or consumption lane keep complete argument deferrals; others
///      propagate them to their enclosing owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallArgumentNumericRetention {
    /// The owner propagates numeric deferrals to its enclosing receiving boundary.
    Propagate,
    /// The owner keeps a complete numeric-dependent argument as a retained list item.
    Retain,
}

/// Typed receiving context for one shared call-argument list.
///
/// WHAT: carries naming, value-evaluation, numeric-retention and diagnostic policy through the one
/// parenthesised list loop.
/// WHY: future named-only/const-required surfaces can select their policy without fabricating
/// declaration slots or introducing a second delimiter parser.
#[derive(Clone, Copy)]
pub(crate) struct CallArgumentReceivingContext {
    pub(crate) naming_policy: CallArgumentNamingPolicy,
    pub(crate) value_policy: CallArgumentValuePolicy,
    pub(crate) numeric_retention: CallArgumentNumericRetention,
    pub(crate) diagnostics: CallArgumentDiagnosticContext,
}

impl CallArgumentReceivingContext {
    pub(crate) fn existing_call(syntax: CallArgumentSyntax) -> Self {
        let naming_policy = match syntax {
            CallArgumentSyntax::Supported { .. }
            | CallArgumentSyntax::Constructor { .. }
            | CallArgumentSyntax::Assertion { .. } => CallArgumentNamingPolicy::PositionalThenNamed,
            CallArgumentSyntax::UnsupportedCall { .. }
            | CallArgumentSyntax::UnsupportedBuiltinMember { .. } => {
                CallArgumentNamingPolicy::PositionalOnly
            }
            CallArgumentSyntax::AnonymousConstRecord => CallArgumentNamingPolicy::NamedOnly,
        };

        Self {
            naming_policy,
            value_policy: CallArgumentValuePolicy::Ordinary,
            numeric_retention: CallArgumentNumericRetention::Propagate,
            diagnostics: CallArgumentDiagnosticContext::from_syntax(syntax),
        }
    }

    pub(crate) fn with_policies(
        diagnostics: CallArgumentDiagnosticContext,
        naming_policy: CallArgumentNamingPolicy,
        value_policy: CallArgumentValuePolicy,
    ) -> Self {
        Self {
            naming_policy,
            value_policy,
            numeric_retention: CallArgumentNumericRetention::Propagate,
            diagnostics,
        }
    }

    /// Opt into retaining arguments for an owner that completes or consumes this list.
    pub(crate) fn with_numeric_retention(
        mut self,
        numeric_retention: CallArgumentNumericRetention,
    ) -> Self {
        self.numeric_retention = numeric_retention;
        self
    }

    pub(crate) fn with_naming_policy(mut self, naming_policy: CallArgumentNamingPolicy) -> Self {
        self.naming_policy = naming_policy;
        self
    }
}

/// One retained-capable call-argument list: fully complete, or holding retained arguments.
///
/// WHAT: separates the two completion states the shared list loop can produce before the caller
///       chooses its lane.
/// WHY: ready lists keep their ordinary `CallArgument` vector; only a real deferral promotes the
///      list to retained-argument storage.
#[derive(Debug, Clone)]
pub(crate) enum ParsedCallArguments<E = ExprId> {
    /// Every argument completed; the list is ready for ordinary validation.
    Ready(Vec<CallArgument<E>>),
    /// At least one argument was retained; completed arguments keep their authored routing.
    Retained(Vec<DeferredCallArgument>),
}

impl<E> ParsedCallArguments<E> {
    pub(crate) fn map_values<T>(self, mut map: impl FnMut(E) -> T) -> ParsedCallArguments<T> {
        match self {
            Self::Ready(arguments) => ParsedCallArguments::Ready(
                arguments
                    .into_iter()
                    .map(|argument| argument.map_value(&mut map))
                    .collect(),
            ),
            Self::Retained(arguments) => ParsedCallArguments::Retained(arguments),
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
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    receiving_context: CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
    syntax_context: CallArgumentSyntaxContext,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    match parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        syntax_context,
        receiving_context,
        expectations,
        path_fork,
    )? {
        ParsedCallArguments::Ready(arguments) => Ok(arguments),
        // This entry has no retained-argument completion lane.
        ParsedCallArguments::Retained(_) => Err(
            crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                "a call surface without numeric-call retention retained a numeric-dependent \
                 argument",
            )
            .into(),
        ),
    }
}

/// Parses a call's argument list while retaining numeric-dependent arguments.
///
/// WHAT: shared entry for receiving owners that keep a complete numeric-dependent argument until
///       their body-root completion or consumption boundary.
/// WHY: argument compatibility cannot run before numeric selection. Retained arguments preserve
///      authored fragments, slot routing, access modes, spans and receiving constraints so the
///      owner resumes the exact authored list without reparsing.
#[allow(
    clippy::too_many_arguments,
    reason = "argument parsing keeps the token stream, scope, type interner, expression builder, const store, string table, expectations, syntax, and path fork as separate borrows"
)]
pub(crate) fn parse_call_arguments_typed_with_numeric_retention(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expectations: &[ParameterExpectation],
    argument_syntax: CallArgumentSyntax,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCallArguments<OwnedExpr>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        CallArgumentSyntaxContext::Ordinary,
        CallArgumentReceivingContext::existing_call(argument_syntax)
            .with_numeric_retention(CallArgumentNumericRetention::Retain),
        Some(expectations),
        path_fork,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "route capture parsing reuses the shared list owner while selecting its positional-prefix naming policy"
)]
pub(crate) fn parse_route_capture_arguments(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expectations: &[ParameterExpectation],
    callee_name: CallDiagnosticName<'static>,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCallArguments<OwnedExpr>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        CallArgumentSyntaxContext::Ordinary,
        CallArgumentReceivingContext::existing_call(CallArgumentSyntax::Constructor {
            callee_name,
        })
        .with_naming_policy(CallArgumentNamingPolicy::PositionalOnly)
        .with_numeric_retention(CallArgumentNumericRetention::Retain),
        Some(expectations),
        path_fork,
    )
}

/// Parses an ordinary argument list when the authored callee has no resolved signature.
///
/// WHAT: consumes named and positional values through the shared parser and router without
///       creating a callee identity, parameter slots, arity or type expectations.
/// WHY: rejected member occurrences still contain authored value expressions whose nested calls
///      must register their own witnesses, but the missing member has no contract to validate.
pub(crate) fn parse_call_arguments_without_expectations(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCallArguments<OwnedExpr>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        CallArgumentSyntaxContext::Ordinary,
        CallArgumentReceivingContext::existing_call(CallArgumentSyntax::Supported {
            callee_name: None,
        })
        .with_numeric_retention(CallArgumentNumericRetention::Retain),
        None,
        path_fork,
    )
}

/// Parses a call argument list with explicit parameter expectations threaded into each argument.
///
/// WHAT: gives every argument expression a `CastTargetContext` derived from its corresponding
///      parameter type, so `cast` can resolve at concrete source/receiver/host
///      parameters and generic parameter slots can reject `cast` with `TargetIsGenericParameter`.
/// WHY: raw call parsing used to resolve arguments before validation. Threading expectations keeps
///      the cast-target channel narrow and local to the argument parser.
#[allow(
    clippy::too_many_arguments,
    reason = "argument parsing keeps the token stream, scope, type interner, expression builder, const store, string table, expectations, syntax, and path fork as separate borrows"
)]
pub(crate) fn parse_call_arguments_typed_with_expectations(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    expectations: &[ParameterExpectation],
    argument_syntax: CallArgumentSyntax,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        CallArgumentReceivingContext::existing_call(argument_syntax),
        Some(expectations),
        CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )
}

#[allow(
    clippy::too_many_arguments,
    reason = "generic argument parsing keeps the token stream, scope, type interner, expression builder, const store, string table, generic callee name, expectations, and path fork as separate borrows"
)]
pub(crate) fn parse_generic_call_arguments_typed(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    generic_function_name: Option<StringId>,
    expectations: &[ParameterExpectation],
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCallArguments<OwnedExpr>, ExpressionParseError> {
    parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        CallArgumentSyntaxContext::GenericFunction {
            function_name: generic_function_name,
        },
        CallArgumentReceivingContext::existing_call(CallArgumentSyntax::Supported {
            callee_name: generic_function_name,
        })
        .with_numeric_retention(CallArgumentNumericRetention::Retain),
        Some(expectations),
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
///      Bare `none` keeps the baseline option slot; any other argument into a numeric
///      (bare or optional) slot carries a literal destination. Every exact numeric receiver
///      (fixed scalar, `Byte`, `Dec`, and their optional forms) qualifies. Named arithmetic
///      stays forward-only: a completed named result keeps its own type.
/// WHY: call arguments otherwise parse with natural-type inference, which is correct for most
///      values but rejects context-sensitive literals before the receiving slot is known.
///      Call validation keeps owning every argument type diagnostic.
fn expected_type_for_parameter_expectation(
    expectation: &ParameterExpectation,
    type_environment: &TypeEnvironment,
    token_stream: &AstCursor,
) -> ExpectedType {
    let type_id = match expectation.expected_type {
        ExpectedParameterType::Known(type_id) => type_id,
        ExpectedParameterType::GenericParameter(type_id) => {
            // WHY: a generic slot never supplies a numeric literal destination. Only a bare `none`
            // keeps its optional destination, so it resolves before generic inference runs.
            return if type_environment.is_option(type_id) && argument_is_bare_none(token_stream) {
                ExpectedType::Known(type_id)
            } else {
                ExpectedType::Infer
            };
        }
        ExpectedParameterType::ConstructionShape(_) | ExpectedParameterType::UnknownExternal => {
            return ExpectedType::Infer;
        }
    };

    if type_environment.is_option(type_id) && argument_is_bare_none(token_stream) {
        return ExpectedType::Known(type_id);
    }

    expected_type_for_resolved_parameter(type_id, type_environment)
}

/// Destination for a retained argument once its parameter type is materialised.
///
/// WHAT: a numeric destination carries its literal context; any other destination keeps the
///       ordinary inference the completed lane already uses.
/// WHY: a construction-shape slot has no type while its argument parses. Completion resumes the
///      retained argument once that slot is materialised and must map it exactly as a concrete
///      numeric slot would.
pub(crate) fn expected_type_for_resolved_parameter(
    type_id: TypeId,
    type_environment: &TypeEnvironment,
) -> ExpectedType {
    if is_numeric_literal_destination_type_id(type_id, type_environment) {
        ExpectedType::NumericLiteral(type_id)
    } else {
        ExpectedType::Infer
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
        ExpectedParameterType::Known(type_id)
        | ExpectedParameterType::GenericParameter(type_id) => {
            cast_target_context_for_type_id(type_id, type_environment, string_table, path_fork)
        }
        ExpectedParameterType::ConstructionShape(_) => {
            // Unresolved shapes never supply a cast target; cast targets stay immediate concrete.
            CastTargetContext::None
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
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    syntax_context: CallArgumentSyntaxContext,
    receiving_context: CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
    path_fork: &mut PathInternerFork,
) -> Result<ParsedCallArguments<OwnedExpr>, ExpressionParseError> {
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
        return Ok(ParsedCallArguments::Ready(Vec::new()));
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
        return Ok(ParsedCallArguments::Ready(Vec::new()));
    }

    let mut arguments = ParsedCallArguments::Ready(Vec::new());
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
            // parser-level ownership truthful by rejecting the positional token directly; record
            // conversion owns field identity validation after the shared list is retained.
            let found = token_stream
                .current_diagnostic_token(string_table)
                .map_err(|error| {
                    CompilerDiagnostic::token_view_invariant_error(
                        error,
                        "named-only argument diagnostic",
                    )
                })?
                .unwrap_or_else(|| DiagnosticToken::from_static_tag(token_stream.current_tag()));
            let diagnostic = if matches!(argument_syntax, CallArgumentSyntax::AnonymousConstRecord)
            {
                CompilerDiagnostic::invalid_expression(
                    InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                    current_span(token_stream),
                )
            } else {
                CompilerDiagnostic::unexpected_token_from_tag(found, argument_span)
            };
            return Err(diagnostic.into());
        }

        let parameter_slot =
            slot_router.route(named_target.as_ref(), argument_span, string_table)?;

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
            let diagnostic = if matches!(argument_syntax, CallArgumentSyntax::AnonymousConstRecord)
            {
                CompilerDiagnostic::invalid_expression(
                    InvalidExpressionReason::AnonymousRecordFieldNotNamed,
                    current_span(token_stream),
                )
            } else {
                CompilerDiagnostic::unexpected_token_from_tag(found, current_span(token_stream))
            };
            return Err(diagnostic.into());
        }

        let parameter_expectation = parameter_slot
            .and_then(|slot| routed_expectations.and_then(|items| items.get(slot.index())));

        let route_argument_context = match parameter_expectation {
            Some(expectation) => match expectation.contract {
                ParameterContract::Route { input } => {
                    let output_type = match expectation.expected_type {
                        ExpectedParameterType::Known(type_id)
                        | ExpectedParameterType::GenericParameter(type_id) => type_id,
                        ExpectedParameterType::ConstructionShape(_)
                        | ExpectedParameterType::UnknownExternal => {
                            return Err(CompilerDiagnostic::invalid_call_shape(
                                InvalidCallShapeReason::RouteArgumentNotDirect,
                                None,
                                current_span(token_stream),
                            )
                            .into());
                        }
                    };
                    if access_mode == CallAccessMode::Mutable
                        || token_stream.current_tag() != TokenTag::SYMBOL
                    {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::RouteArgumentNotDirect,
                            None,
                            current_span(token_stream),
                        )
                        .into());
                    }
                    Some(crate::compiler_frontend::ast::expressions::parse_expression_input::RouteArgumentContext {
                        output_type,
                        input_type: input,
                    })
                }
                ParameterContract::Value | ParameterContract::Wire => None,
            },
            None => None,
        };
        if parameter_expectation
            .is_some_and(|expectation| expectation.contract == ParameterContract::Wire)
            && access_mode == CallAccessMode::Mutable
        {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::WireArgumentMutableMarker,
                None,
                marker_span,
            )
            .into());
        }
        // A construction-shape slot keeps its literal unresolved until completion, and its ready
        // and retained arguments join the slot's numeric class.
        let construction_shape =
            parameter_expectation.and_then(|expectation| match expectation.expected_type {
                ExpectedParameterType::ConstructionShape(shape) => Some(shape),
                ExpectedParameterType::Known(_)
                | ExpectedParameterType::GenericParameter(_)
                | ExpectedParameterType::UnknownExternal => None,
            });
        let mut inferred = parameter_expectation
            .map(|expectation| {
                expected_type_for_parameter_expectation(
                    expectation,
                    type_interner.environment(),
                    token_stream,
                )
            })
            .unwrap_or(ExpectedType::Infer);
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
        // A Retain argument into a concrete collection or map slot, or into one optional layer over
        // either, keeps its pending fragment, so the slot's exact receiver types the fragment's
        // literal leaves before body-root defaults run. The optional layer peels exactly as the
        // receiver witness peels it. A generic-parameter slot is handled by `generic_slot` below.
        let retains_known_aggregate =
            matches!(
                receiving_context.numeric_retention,
                CallArgumentNumericRetention::Retain
            ) && parameter_expectation.is_some_and(|expectation| match expectation.expected_type {
                ExpectedParameterType::Known(type_id) => {
                    let environment = type_interner.environment();
                    let receiver = environment.option_inner_type(type_id).unwrap_or(type_id);
                    environment.collection_shape(receiver).is_some()
                        || environment.map_shape(receiver).is_some()
                }
                ExpectedParameterType::ConstructionShape(_)
                | ExpectedParameterType::GenericParameter(_)
                | ExpectedParameterType::UnknownExternal => false,
            });
        // Every generic-parameter slot retains its pending fragment, so the completed call can link the
        // fragment's numeric origin to its declared generic parameter before any default applies.
        let generic_slot = parameter_expectation.is_some_and(|expectation| {
            matches!(
                expectation.expected_type,
                ExpectedParameterType::GenericParameter(_)
            )
        });
        // The argument's authored extent is needed by the Wire-origin check below. The expression
        // parse consumes the argument's trailing newlines, so the token position cannot describe
        // the argument: capture the first token's span and compare the parsed expression's span
        // against it instead.
        let argument_source_span = token_stream.current_span();
        let argument_is_bare_symbol = token_stream.current_tag() == TokenTag::SYMBOL;
        let route_forwarding_value = if route_argument_context.is_some() && argument_is_bare_symbol
        {
            let name = token_stream
                .current_string_id_in(string_table)?
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "route argument symbol token had no string payload",
                    )
                })?;
            let route_reference: Option<RouteParameterReference<'_>> =
                context.get_route_argument_reference(&name);
            match route_reference {
                Some(route_reference) => {
                    let route_context = route_argument_context.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "Route parameter lookup escaped its route argument context",
                        )
                    })?;
                    let declaration = route_reference.declaration();
                    if declaration.type_id != route_context.output_type
                        || route_reference.input_type() != route_context.input_type
                    {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::RouteForwardingContractMismatch,
                            None,
                            Some(argument_source_span),
                        )
                        .into());
                    }

                    token_stream.advance();
                    token_stream.skip_newlines();
                    if !matches!(
                        token_stream.current_tag(),
                        TokenTag::COMMA | TokenTag::CLOSE_PARENTHESIS
                    ) {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::RouteArgumentNotDirect,
                            None,
                            Some(argument_source_span),
                        )
                        .into());
                    }

                    Some(reference_expression_from_declaration(
                        declaration,
                        context,
                        type_interner,
                        expression_builder,
                        const_values,
                        string_table,
                        Some(argument_source_span),
                    )?)
                }
                None => {
                    match context.get_reference(&name) {
                        Some(ScopeDeclarationRef::PendingNumeric { .. }) => {
                            return Err(CompilerDiagnostic::invalid_call_shape(
                                InvalidCallShapeReason::RouteArgumentNotDirect,
                                None,
                                Some(argument_source_span),
                            )
                            .into());
                        }
                        Some(ScopeDeclarationRef::Local { .. })
                        | Some(ScopeDeclarationRef::Shared(_))
                        | Some(ScopeDeclarationRef::RouteParameter(_))
                        | None => {}
                    }
                    None
                }
            }
        } else {
            None
        };
        let is_route_forwarding_argument = route_forwarding_value.is_some();
        // Only an argument list that can retain a deferral treats its arguments as owners of their
        // own complete fragment. Every other surface keeps raising non-owning deferrals, so a
        // nested parse that swallowed enclosing structure can never be consumed as a whole
        // argument by an outer retention site.
        let producer_start = context.numeric_state().forward_producer_checkpoint();
        let argument_result = if let Some(value) = route_forwarding_value {
            Ok(value)
        } else {
            let mut input = ExpressionParseInput::call_argument(
                ExpressionParseResources {
                    token_stream,
                    scope_context: context,
                    type_interner,
                    expression_builder,
                    const_values,
                    expected_type: &mut inferred,
                    cast_target_context: &mut cast_target_context,
                    value_mode: &ValueMode::ImmutableOwned,
                    string_table,
                    path_fork,
                },
                matches!(
                    receiving_context.numeric_retention,
                    CallArgumentNumericRetention::Retain
                ),
            );
            if let Some(route_argument_context) = route_argument_context {
                input = input.with_route_argument_context(route_argument_context);
            }
            if construction_shape.is_some() || retains_known_aggregate || generic_slot {
                input = input.with_retained_pending_numeric_fragment();
            }

            if matches!(
                receiving_context.naming_policy,
                CallArgumentNamingPolicy::NamedOnly
            ) {
                input.stop_at_named_entry = true;
            }

            create_expression_with_trailing_newline_policy(input)
        };
        let argument = match argument_result {
            Ok(value) => {
                if route_argument_context.is_some() {
                    let argument_view = expression_builder.view_owned(value)?;
                    let direct_route_root = match argument_view.operation() {
                        ExpressionOperationView::RouteConstruct { .. } => true,
                        ExpressionOperationView::Reference(_) => {
                            is_route_forwarding_argument
                                && argument_view.span() == Some(argument_source_span)
                        }
                        _ => false,
                    };
                    if !direct_route_root {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::RouteArgumentNotDirect,
                            None,
                            argument_view.span(),
                        )
                        .into());
                    }
                }
                if parameter_expectation
                    .is_some_and(|expectation| expectation.contract == ParameterContract::Wire)
                {
                    let argument_view = expression_builder.view_owned(value)?;
                    if !wire_argument_origin_is_eligible(
                        argument_view,
                        argument_is_bare_symbol,
                        argument_source_span,
                        context,
                        path_fork,
                    ) {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::WireArgumentOrigin,
                            None,
                            argument_view.span(),
                        )
                        .into());
                    }
                }
                validate_argument_value_policy(
                    const_values,
                    string_table,
                    expression_builder,
                    expression_builder.view_owned(value)?,
                    context,
                    path_fork,
                    receiving_context,
                )?;
                // Preserve the value-expression span, named-parameter span, and authored `~`
                // marker span independently so diagnostics can point at whichever source the
                // author must change.
                let value_span = expression_builder.view_owned(value)?.span();
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

                if let Some(shape) = construction_shape {
                    let mut state = context.numeric_state_mut();
                    register_shape_witness(
                        &mut state,
                        type_interner.environment(),
                        shape,
                        expression_builder
                            .view_owned(argument.value)?
                            .value_type_id()?,
                        argument.span,
                    );
                }

                DeferredCallArgument::Ready(argument)
            }
            Err(error) => {
                if matches!(
                    receiving_context.numeric_retention,
                    CallArgumentNumericRetention::Propagate
                ) {
                    return Err(error);
                }

                // Only the argument's own complete fragment may be retained: a nested parse that
                // swallowed enclosing argument structure would otherwise lose that structure.
                let deferral = error.take_numeric_deferral("a call argument")?;
                if route_argument_context.is_some()
                    && !retained_route_argument_is_constructor_root(&deferral.items)
                {
                    return Err(CompilerDiagnostic::invalid_call_shape(
                        InvalidCallShapeReason::RouteArgumentNotDirect,
                        None,
                        deferral.span,
                    )
                    .into());
                }
                // Deferred expressions have no completed operation view; only one pending
                // authored binding can still be the direct Wire source.
                if parameter_expectation
                    .is_some_and(|expectation| expectation.contract == ParameterContract::Wire)
                    && !(argument_is_bare_symbol
                        && matches!(
                            deferral.items.as_slice(),
                            [ExpressionRpnItem::PendingNumericBinding { is_copy: false, .. }]
                        ))
                {
                    return Err(CompilerDiagnostic::invalid_call_shape(
                        InvalidCallShapeReason::WireArgumentOrigin,
                        None,
                        deferral.span,
                    )
                    .into());
                }
                // A known non-numeric parameter is the exact receiver of its argument's direct origin,
                // so its shape constrains the origin's numeric leaves before body-root defaults. A
                // numeric parameter already registers through its literal destination.
                let known_receiver =
                    parameter_expectation.and_then(|expectation| match expectation.expected_type {
                        ExpectedParameterType::Known(type_id) => Some(type_id),
                        ExpectedParameterType::ConstructionShape(_)
                        | ExpectedParameterType::GenericParameter(_)
                        | ExpectedParameterType::UnknownExternal => None,
                    });
                if let Some(receiver_type_id) = known_receiver.filter(|type_id| {
                    !is_numeric_literal_destination_type_id(*type_id, type_interner.environment())
                }) {
                    // The argument's carrier leaves through its call's own error lane, so the call owns
                    // any unhandled step there; the success value still constrains the parameter.
                    register_direct_receiver_witness(
                        context,
                        type_interner.environment(),
                        expression_builder,
                        &deferral.items,
                        receiver_type_id,
                        true,
                    )?;
                    // The argument's own producers take the parameter's exact receiver, so its
                    // aggregate leaves are typed by the parameter, not by defaults or later witnesses.
                    let owned_producers =
                        producer_start..context.numeric_state().forward_producer_checkpoint();
                    let fragment = fragment_shape(
                        context,
                        type_interner.environment_mut_for_derived_types(),
                        &deferral.items,
                        expression_builder,
                    )?;
                    let mut state = context.numeric_state_mut();
                    set_forward_destinations(
                        &mut state,
                        type_interner.environment(),
                        fragment,
                        receiver_type_id,
                        &owned_producers,
                    );
                }
                // A construction-shape slot joins the retained fragment's class before any default
                // applies, so its literal shares the slot's numeric family. A forward fragment in
                // that class registers its own producer, so its natural result completes before the
                // class defaults.
                // A generic slot instead keeps its fragment as the origin of a declared generic
                // relation, linked at completion. Only a forward fragment registers a producer here.
                let (forward, origin_shape) = match construction_shape {
                    Some(shape) => {
                        let fragment = fragment_shape(
                            context,
                            type_interner.environment_mut_for_derived_types(),
                            &deferral.items,
                            expression_builder,
                        )?;
                        let mut state = context.numeric_state_mut();
                        // The forward flag is read before the union, which ORs a forward shape's
                        // flag into the class; a raw fragment must not register as a producer.
                        let class = state.forward_numeric_class(fragment);
                        union_shape_classes(
                            &mut state,
                            type_interner.environment(),
                            fragment,
                            shape,
                        );
                        (class.map(|class| (class, fragment)), None)
                    }
                    None if generic_slot => {
                        let fragment = fragment_shape(
                            context,
                            type_interner.environment_mut_for_derived_types(),
                            &deferral.items,
                            expression_builder,
                        )?;
                        let class = context.numeric_state_mut().forward_numeric_class(fragment);
                        (class.map(|class| (class, fragment)), Some(fragment))
                    }
                    None => (None, None),
                };

                let (target_param, target_span) = match named_target {
                    Some((name, span)) => (Some(name), span),
                    None => (None, None),
                };
                let argument_shape = DeferredCallArgumentShape {
                    target_param,
                    access_mode,
                    marker_span,
                    value_span: deferral.span,
                    target_span,
                    parameter_slot,
                    following_separator_span: None,
                    origin_shape,
                };
                match forward {
                    Some((class, fragment)) => {
                        let typed_errors =
                            fragment_typed_error_summary(expression_builder, &deferral.items)?;
                        let expression = deferred_numeric_expression(
                            deferral.items,
                            deferral.expected,
                            deferral.value_mode,
                            deferral.span,
                            deferral.suffix,
                        );
                        let producer = context
                            .numeric_state_mut()
                            .register_forward_producer(class, fragment, expression)?;
                        DeferredCallArgument::Forward {
                            shape: argument_shape,
                            producer,
                            typed_errors,
                        }
                    }
                    None => DeferredCallArgument::Retained {
                        shape: argument_shape,
                        expression: deferred_numeric_expression(
                            deferral.items,
                            deferral.expected,
                            deferral.value_mode,
                            deferral.span,
                            deferral.suffix,
                        ),
                    },
                }
            }
        };

        let (argument, list_ended) = match token_stream.current_tag() {
            TokenTag::COMMA => {
                let separator_span = current_span(token_stream);
                token_stream.advance();
                token_stream.skip_newlines();
                let argument = with_following_separator_span(argument, separator_span);
                (argument, false)
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
                let diagnostic = if matches!(
                    receiving_context.naming_policy,
                    CallArgumentNamingPolicy::NamedOnly
                ) && current_token_starts_named_entry(token_stream)
                {
                    CompilerDiagnostic::expected_token_from_tags(
                        TokenTag::COMMA,
                        Some(found),
                        current_span(token_stream),
                    )
                } else {
                    CompilerDiagnostic::unexpected_token_from_tag(found, current_span(token_stream))
                };
                return Err(diagnostic.into());
            }
        };

        arguments = match (arguments, argument) {
            (ParsedCallArguments::Ready(mut ready), DeferredCallArgument::Ready(argument)) => {
                ready.push(argument);
                ParsedCallArguments::Ready(ready)
            }
            (
                ParsedCallArguments::Ready(ready),
                retained @ (DeferredCallArgument::Retained { .. }
                | DeferredCallArgument::Forward { .. }),
            ) => {
                let mut retained_arguments = Vec::with_capacity(ready.len() + 1);
                retained_arguments.extend(ready.into_iter().map(|argument| {
                    DeferredCallArgument::Ready(argument.map_value(OwnedExpr::id))
                }));
                retained_arguments.push(retained.map_value(OwnedExpr::id));
                ParsedCallArguments::Retained(retained_arguments)
            }
            (ParsedCallArguments::Retained(mut retained), argument) => {
                retained.push(argument.map_value(OwnedExpr::id));
                ParsedCallArguments::Retained(retained)
            }
        };
        if list_ended {
            break;
        }
    }

    Ok(arguments)
}

/// Records the authored comma span on a completed or retained argument.
///
/// WHY: both argument shapes carry the separator only for diagnostics, and the retained shape
///      must keep it as the authored routing fact instead of a rebuilt argument.
fn with_following_separator_span<E>(
    argument: DeferredCallArgument<E>,
    separator_span: Option<SourceSpan>,
) -> DeferredCallArgument<E> {
    match argument {
        DeferredCallArgument::Ready(argument) => {
            DeferredCallArgument::Ready(argument.with_following_separator_span(separator_span))
        }
        DeferredCallArgument::Retained {
            mut shape,
            expression,
        } => {
            shape.following_separator_span = separator_span;
            DeferredCallArgument::Retained { shape, expression }
        }
        DeferredCallArgument::Forward {
            mut shape,
            producer,
            typed_errors,
        } => {
            shape.following_separator_span = separator_span;
            DeferredCallArgument::Forward {
                shape,
                producer,
                typed_errors,
            }
        }
    }
}

/// Preserve the direct-constructor obligation when a route constructor waits for numeric completion.
fn retained_route_argument_is_constructor_root(items: &[ExpressionRpnItem]) -> bool {
    matches!(
        items,
        [ExpressionRpnItem::PendingNumericCall { call, .. }]
            if matches!(
                call.as_ref(),
                DeferredCall::Choice(choice) if choice.is_route
            )
    )
}

/// Whether one parsed `Wire` argument names an eligible authored binding.
///
/// WHAT: accepts exactly one authored symbol whose parsed value is a single `Reference` naming
///       either a `Wire` parameter of the enclosing callable or a runtime body-local binding that
///       is not a compile-time constant.
/// WHY: a `Wire` argument transmits an existing value binding, so its source must be a binding
///      rather than a computed value, must be runtime state rather than compile-time content, and
///      must not be an ordinary parameter (which carries no transmitted binding). The argument's
///      authored extent is its first token's span compared against the parsed expression's span,
///      because the expression parse consumes trailing newlines and a token-position count would
///      reject a call whose argument list spans lines.
fn wire_argument_origin_is_eligible(
    argument_view: ExpressionView<'_>,
    argument_is_bare_symbol: bool,
    argument_source_span: SourceSpan,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
) -> bool {
    if !argument_is_bare_symbol || argument_view.span() != Some(argument_source_span) {
        return false;
    }

    let ExpressionOperationView::Reference(path) = argument_view.operation() else {
        return false;
    };

    if let Some(contract) = context
        .enclosing_callable
        .as_ref()
        .and_then(|callable| callable.parameter_contract(path))
    {
        // A parameter of the callable being parsed is not an authored body binding: only the
        // callable's own `Wire` parameter forwards the capability. Ordinary and `Route`
        // parameters stop here instead of reaching the local-binding arm through their
        // default-less `Absent` value.
        return contract == ParameterContract::Wire;
    }

    path_fork.component(path).is_some_and(|name| {
        context
            .get_reference(&name)
            .is_some_and(|reference| match reference {
                ScopeDeclarationRef::Local {
                    declaration,
                    binding_span,
                } => {
                    declaration.id == path
                        && binding_span.is_some()
                        && declaration.binding_span.is_some()
                        && matches!(
                            &declaration.value,
                            DeclarationValue::Initializer(_) | DeclarationValue::Absent
                        )
                        && !context.is_explicit_compile_time_constant(&declaration)
                }
                ScopeDeclarationRef::Shared(_)
                | ScopeDeclarationRef::PendingNumeric { .. }
                | ScopeDeclarationRef::RouteParameter(_) => false,
            })
    })
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
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    expression_builder: &AstExpressionBuilder,
    value: ExpressionView<'_>,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
    receiving_context: CallArgumentReceivingContext,
) -> Result<(), ExpressionParseError> {
    let receiver = match receiving_context.diagnostics.syntax {
        CallArgumentSyntax::Constructor { .. } | CallArgumentSyntax::AnonymousConstRecord => {
            ValueReceiverContext::ConstructorArgument
        }
        CallArgumentSyntax::Assertion { .. } => ValueReceiverContext::AssertionArgument,
        _ => ValueReceiverContext::FunctionArgument,
    };
    value.require_single(receiver)?;
    if !matches!(
        receiving_context.value_policy,
        CallArgumentValuePolicy::ConstRequired
    ) {
        return Ok(());
    }

    let is_placeholder_reference =
        if let ExpressionOperationView::Reference(path) = value.operation() {
            path_fork
                .component(path)
                .map(|name| context.get_value_reference(&name, value.span()))
                .transpose()?
                .flatten()
                .and_then(|reference| {
                    reference
                        .ready_declaration()
                        .map(|declaration| context.is_unresolved_constant_placeholder(declaration))
                })
                .unwrap_or(false)
        } else {
            false
        };

    let is_compile_time_value = is_placeholder_reference
        || value
            .const_value_kind_with_template_classifier(const_values, &mut |template| {
                classify_template_from_effective_tir(
                    template,
                    &context.template_ir_store,
                    expression_builder,
                    const_values,
                )
            })?
            .is_compile_time_value();

    if is_compile_time_value {
        return Ok(());
    }

    Err(CompilerDiagnostic::compile_time_evaluation_error(
        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable,
        receiving_context
            .diagnostics
            .const_operation
            .and_then(|operation| operation.diagnostic_callee_name(string_table)),
        value.span(),
    )
    .into())
}

struct ParameterSlotRouter<'a> {
    expectations: Option<&'a [ParameterExpectation]>,
    receiving_context: CallArgumentReceivingContext,
    parameter_name_to_slot: FxHashMap<StringId, usize>,
    named_target_spans: Option<FxHashMap<StringId, Option<SourceSpan>>>,
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
        let named_target_spans = matches!(
            receiving_context.naming_policy,
            CallArgumentNamingPolicy::NamedOnly
        )
        .then(FxHashMap::default);

        Self {
            expectations,
            receiving_context,
            parameter_name_to_slot,
            named_target_spans,
            positional_cursor: 0,
            saw_named_argument: false,
            occupied_parameter_slots: expectations.map(|items| vec![false; items.len()]),
        }
    }

    fn route(
        &mut self,
        named_target: Option<&(StringId, Option<SourceSpan>)>,
        argument_span: Option<SourceSpan>,
        string_table: &mut StringTable,
    ) -> Result<Option<ParameterSlot>, ExpressionParseError> {
        if let Some(named_target_spans) = self.named_target_spans.as_mut()
            && let Some((target_name, target_span)) = named_target
        {
            if let Some(first_span) = named_target_spans.get(target_name) {
                return Err(CompilerDiagnostic::duplicate_declaration(
                    *target_name,
                    *first_span,
                    *target_span,
                )
                .into());
            }
            named_target_spans.insert(*target_name, *target_span);
        }

        if let Some((_, target_span)) = named_target
            && matches!(
                self.receiving_context.naming_policy,
                CallArgumentNamingPolicy::PositionalOnly
            )
        {
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
                CallArgumentSyntax::Supported { .. }
                | CallArgumentSyntax::Constructor { .. }
                | CallArgumentSyntax::Assertion { .. } => {
                    return Err(CompilerDiagnostic::invalid_call_shape(
                        InvalidCallShapeReason::NamedArgumentsNotSupported,
                        self.callee_name(string_table),
                        *target_span,
                    )
                    .into());
                }
                CallArgumentSyntax::AnonymousConstRecord => {}
            }
        }

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
                CallArgumentSyntax::Supported { .. }
                | CallArgumentSyntax::Constructor { .. }
                | CallArgumentSyntax::Assertion { .. } => {
                    let Some(slot) = self.parameter_name_to_slot.get(target_name).copied() else {
                        return Err(CompilerDiagnostic::invalid_call_shape(
                            InvalidCallShapeReason::NamedArgumentNotFound {
                                name: *target_name,
                                known_parameters: known_parameter_names(expectations),
                            },
                            self.callee_name(string_table),
                            *target_span,
                        )
                        .into());
                    };

                    self.mark_slot_occupied(slot, *target_span, string_table)?;
                    return Ok(Some(ParameterSlot::new(slot)));
                }
                CallArgumentSyntax::AnonymousConstRecord => return Ok(None),
            }
        }

        if self.saw_named_argument {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::PositionalAfterNamed,
                self.callee_name(string_table),
                argument_span,
            )
            .into());
        }

        let Some(slot_count) = self.declared_slot_count() else {
            let slot = self.positional_cursor;
            self.positional_cursor += 1;
            return Ok(Some(ParameterSlot::new(slot)));
        };

        while self.positional_cursor < slot_count && self.slot_is_occupied(self.positional_cursor) {
            self.positional_cursor += 1;
        }

        if self.positional_cursor >= slot_count {
            return Err(CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::ExtraPositionalArgument {
                    expected_count: expectations.len(),
                },
                self.callee_name(string_table),
                argument_span,
            )
            .into());
        }

        let slot = self.positional_cursor;
        self.occupy_slot(slot);
        self.positional_cursor += 1;
        Ok(Some(ParameterSlot::new(slot)))
    }

    fn mark_slot_occupied(
        &mut self,
        slot: usize,
        span: Option<SourceSpan>,
        string_table: &mut StringTable,
    ) -> Result<(), ExpressionParseError> {
        let parameter_name = self
            .expectations
            .and_then(|items| items.get(slot))
            .and_then(|expectation| expectation.name);

        if self.slot_is_occupied(slot) {
            let diagnostic = CompilerDiagnostic::invalid_call_shape(
                InvalidCallShapeReason::DuplicateArgument {
                    parameter_name,
                    parameter_index: slot,
                },
                self.callee_name(string_table),
                span,
            );
            return Err(diagnostic.into());
        }

        self.occupy_slot(slot);
        Ok(())
    }

    /// The parameter slots this list routes, when the receiving surface declares them.
    ///
    /// WHAT: `None` marks a surface with no declaration-order slots, whose positional arguments
    ///       carry no routing decision.
    fn declared_slot_count(&self) -> Option<usize> {
        self.occupied_parameter_slots.as_ref().map(Vec::len)
    }

    /// Whether a declared slot already holds an argument.
    fn slot_is_occupied(&self, slot: usize) -> bool {
        self.occupied_parameter_slots
            .as_ref()
            .is_some_and(|occupied_slots| occupied_slots[slot])
    }

    /// Marks a declared slot occupied after this list accepted an argument for it.
    fn occupy_slot(&mut self, slot: usize) {
        if let Some(occupied_slots) = self.occupied_parameter_slots.as_mut() {
            occupied_slots[slot] = true;
        }
    }

    fn callee_name(&self, string_table: &mut StringTable) -> Option<StringId> {
        match self.receiving_context.diagnostics.syntax {
            CallArgumentSyntax::Supported { callee_name }
            | CallArgumentSyntax::Assertion { callee_name }
            | CallArgumentSyntax::UnsupportedCall { callee_name } => callee_name,
            CallArgumentSyntax::Constructor { callee_name } => {
                callee_name.diagnostic_callee_name(string_table)
            }
            CallArgumentSyntax::UnsupportedBuiltinMember { .. }
            | CallArgumentSyntax::AnonymousConstRecord => None,
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

fn current_token_starts_named_entry(token_stream: &AstCursor) -> bool {
    token_stream.current_tag() == TokenTag::SYMBOL
        && next_token_tag(token_stream) == Some(TokenTag::ASSIGN)
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
    ) && (type_tag.is_builtin_scalar_type_name() || type_tag == TokenTag::DATATYPE_NONE)
        && matches!(
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
