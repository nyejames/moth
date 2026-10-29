//! Function call argument parser and source-span regression tests.
//!
//! WHAT: protects raw parsed argument shape, call-access mode classification, and the distinct
//! named-target, value-expression and authored-marker spans produced by the call argument parser.
//! WHY: parser-local facts and the parser-to-final-validation retained-slot handoff are internal
//! invariants that end-to-end integration output cannot inspect. Whole-source call acceptance
//! and rejection behavior is owned by canonical integration cases under
//!      `tests/cases/function_call_*`.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, CallDiagnosticName, ExpectedAccessMode,
    ExpectedParameterType, ParameterDefault, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    ExpressionOperationView, OwnedExpr,
};
use crate::compiler_frontend::ast::statements::functions::ParameterContract;
use crate::compiler_frontend::ast::{
    AstTypeInterner, ContextKind, ScopeContext, TopLevelDeclarationTable,
};
use crate::compiler_frontend::compiler_messages::TypeMismatchContext;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidCallShapeCode, InvalidCallShapeView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow::CompileTimeEvaluationErrorView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    UnexpectedTokenShape, UnexpectedTokenView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::compact_bag_for_test;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticBag, DiagnosticDraft, DiagnosticToken,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_primary_span, ast_first_error_rendered_message,
    first_ast_error_view, parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{TokenTag, TokenizerEntryMode};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::rc::Rc;
use std::sync::Arc;

fn ast_const_payload_is_fixed_scalar(
    const_values: &crate::compiler_frontend::ast::const_values::store::ConstValueStore,
    id: crate::compiler_frontend::ast::const_values::store::ConstValueId,
    expected: FixedScalarValue,
) -> bool {
    matches!(const_values.payload(id), Some(crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(value)) if *value == expected)
}

type OwnedTestArguments = (
    Vec<CallArgument<OwnedExpr>>,
    crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder,
    crate::compiler_frontend::ast::const_values::store::ConstValueStore,
);
fn parse_args(source: &str) -> Vec<CallArgument<OwnedExpr>> {
    parse_owned_args_with_receiving_context(
        source,
        super::CallArgumentReceivingContext::existing_call(super::CallArgumentSyntax::Supported {
            callee_name: None,
        }),
        None,
    )
    .expect("call arguments should parse")
    .0
}
fn parse_args_with_receiving_context(
    source: &str,
    receiving_context: super::CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    parse_owned_args_with_receiving_context(source, receiving_context, expectations)
        .map(|owned| owned.0)
}
fn parse_owned_args_with_receiving_context(
    source: &str,
    receiving_context: super::CallArgumentReceivingContext,
    expectations: Option<&[ParameterExpectation]>,
) -> Result<OwnedTestArguments, ExpressionParseError> {
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let tokens = tokenize(
        source,
        file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let range = tokens
        .tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&tokens.tokens, range)
        .expect("test token stream must expose an AST cursor");

    while token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        token_stream.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut expression_builder =
        crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder::new()
            .unwrap();
    let mut const_values =
        crate::compiler_frontend::ast::const_values::store::ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    super::parse_call_arguments_with_receiving_context(
        &mut token_stream,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        receiving_context,
        expectations,
        super::CallArgumentSyntaxContext::Ordinary,
        &mut path_fork,
    )
    .map(|arguments| (arguments, expression_builder, const_values))
}

fn parse_args_with_parameter_names(
    source: &str,
    parameter_names: &[&str],
) -> Vec<CallArgument<OwnedExpr>> {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let range = tokens
        .tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&tokens.tokens, range)
        .expect("test token stream must expose an AST cursor");

    while token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        token_stream.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let expectations = parameter_names
        .iter()
        .map(|name| ParameterExpectation {
            name: Some(string_table.intern(name)),
            expected_type: ExpectedParameterType::UnknownExternal,
            access_mode: ExpectedAccessMode::Shared,
            default_value: None,
            contract: ParameterContract::Value,
        })
        .collect::<Vec<_>>();

    let mut expression_builder =
        crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder::new()
            .unwrap();
    let mut const_values =
        crate::compiler_frontend::ast::const_values::store::ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    super::parse_call_arguments_typed_with_expectations(
        &mut token_stream,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &expectations,
        super::CallArgumentSyntax::Supported { callee_name: None },
        &mut path_fork,
    )
    .expect("call arguments should parse")
}

/// Parses raw call arguments without parameter expectations for syntax-level tests.
///
/// WHAT: calls the production argument parser with no expectations so syntax-only tests are not
///       coupled to parameter counts or types.
/// WHY: keeping this local to the test module avoids a test-only production API while preserving
///      the exact behavior these syntax tests need.
fn parse_raw_call_args_for_test(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder,
    const_values: &mut crate::compiler_frontend::ast::const_values::store::ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<CallArgument<OwnedExpr>>, ExpressionParseError> {
    super::parse_call_arguments_with_receiving_context(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        super::CallArgumentReceivingContext::existing_call(super::CallArgumentSyntax::Supported {
            callee_name: None,
        }),
        None,
        super::CallArgumentSyntaxContext::Ordinary,
        path_fork,
    )
}

fn parse_args_diagnostic(source: &str) -> DiagnosticDraft {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        source,
        file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let range = tokens
        .tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&tokens.tokens, range)
        .expect("test token stream must expose an AST cursor");

    while token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        token_stream.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut expression_builder =
        crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder::new()
            .unwrap();
    let mut const_values =
        crate::compiler_frontend::ast::const_values::store::ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let error = parse_raw_call_args_for_test(
        &mut token_stream,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &mut path_fork,
    )
    .expect_err("call arguments should fail");
    match error {
        ExpressionParseError::Diagnostic(diagnostic)
        | ExpressionParseError::TemplateDirectiveEmptyArguments(diagnostic)
        | ExpressionParseError::NonConstantReferenceInConstant(diagnostic) => diagnostic,
        ExpressionParseError::Infrastructure(error) => {
            panic!("expected user diagnostic, found infrastructure error: {error:?}")
        }
        ExpressionParseError::NumericDeferral(deferral) => {
            panic!("expected user diagnostic, found numeric deferral: {deferral:?}")
        }
    }
}

// ── Parser-level tests (syntax-only call arguments) ──────────────────────────

#[test]
fn parses_positional_and_named_call_arguments_with_equals_syntax() {
    let args = parse_args("sum(1, b = 2)");

    assert_eq!(args.len(), 2);
    assert!(args[0].target_param.is_none());
    assert_eq!(args[0].access_mode, CallAccessMode::Shared);
    assert!(args[1].target_param.is_some());
}

#[test]
fn parses_named_mutable_argument_on_value_side() {
    let args = parse_args("take(value = ~1)");

    assert_eq!(args.len(), 1);
    assert!(args[0].target_param.is_some());
    assert_eq!(args[0].access_mode, CallAccessMode::Mutable);
}

#[test]
fn call_argument_spans_keep_named_target_value_and_marker_distinct() {
    // `parameter = ~value` must keep the named-target token, the value expression and the
    // authored `~` marker at three distinct source spans so diagnostics can label the source the
    // author must change. The value here is a literal, not a binding.
    let args = parse_args("take(value = ~1)");

    assert_eq!(args.len(), 1);
    let argument = &args[0];
    assert_eq!(argument.access_mode, CallAccessMode::Mutable);

    let target_span = argument
        .target_span
        .expect("named argument should carry a target span");
    let marker_span = argument
        .marker_span
        .expect("authored ~ should carry a marker span");

    assert_ne!(argument.span, Some(target_span));
    assert_ne!(argument.span, Some(marker_span));
    assert_ne!(Some(target_span), Some(marker_span));
}

#[test]
fn call_argument_marker_span_is_absent_without_authored_tilde() {
    let args = parse_args("take(value = 1)");

    assert_eq!(args.len(), 1);
    assert_eq!(args[0].access_mode, CallAccessMode::Shared);
    assert!(
        args[0].marker_span.is_none(),
        "absent ~ must not synthesize a marker span",
    );
}

#[test]
fn parses_all_named_arguments() {
    let args = parse_args("sum(a = 1, b = 2)");

    assert_eq!(args.len(), 2);
    assert!(args[0].target_param.is_some());
    assert!(args[1].target_param.is_some());
}

#[test]
fn parses_mixed_positional_then_named() {
    let args = parse_args("sum(1, b = 2, c = 3)");

    assert_eq!(args.len(), 3);
    assert!(args[0].target_param.is_none());
    assert!(args[1].target_param.is_some());
    assert!(args[2].target_param.is_some());
}

#[test]
fn named_only_receiving_context_keeps_labels_without_signature_slots() {
    let diagnostics =
        super::CallArgumentDiagnosticContext::from_syntax(super::CallArgumentSyntax::Supported {
            callee_name: None,
        });
    let receiving_context = super::CallArgumentReceivingContext::with_policies(
        diagnostics,
        super::CallArgumentNamingPolicy::NamedOnly,
        super::CallArgumentValuePolicy::Ordinary,
    );
    let args =
        parse_args_with_receiving_context("record(first = 1, second = 2)", receiving_context, None)
            .expect("named-only arguments should parse");

    assert_eq!(args.len(), 2);
    assert!(args.iter().all(|argument| argument.target_param.is_some()));
    assert!(
        args.iter()
            .all(|argument| argument.parameter_slot.is_none()),
        "named-only values must not fabricate declaration-order slots",
    );
}

#[test]
fn named_only_receiving_context_rejects_positional_arguments() {
    let diagnostics =
        super::CallArgumentDiagnosticContext::from_syntax(super::CallArgumentSyntax::Supported {
            callee_name: None,
        });
    let receiving_context = super::CallArgumentReceivingContext::with_policies(
        diagnostics,
        super::CallArgumentNamingPolicy::NamedOnly,
        super::CallArgumentValuePolicy::Ordinary,
    );
    let error = parse_args_with_receiving_context("record(1)", receiving_context, None)
        .expect_err("named-only arguments must reject positional values");

    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected a user diagnostic for positional named-only input");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic
            .family_view::<UnexpectedTokenView>()
            .expect("the diagnostic is unexpected-token"),
        facts if matches!(facts.shape, UnexpectedTokenShape::Token(_))
    ));
}

#[test]
fn positional_only_without_signature_rejects_named_arguments() {
    let diagnostics =
        super::CallArgumentDiagnosticContext::from_syntax(super::CallArgumentSyntax::Supported {
            callee_name: None,
        });
    let receiving_context = super::CallArgumentReceivingContext::with_policies(
        diagnostics,
        super::CallArgumentNamingPolicy::PositionalOnly,
        super::CallArgumentValuePolicy::Ordinary,
    );
    let error = parse_args_with_receiving_context("directive(name = 1)", receiving_context, None)
        .expect_err("positional-only receivers must reject named arguments without a signature");

    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected a user diagnostic for a named positional-only argument");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    let facts = diagnostic
        .family_view::<InvalidCallShapeView>()
        .expect("the diagnostic is an invalid call shape");
    assert_eq!(
        facts.reason,
        InvalidCallShapeCode::NamedArgumentsNotSupported
    );
}

#[test]
fn const_required_receiving_context_reuses_expression_const_classification() {
    let diagnostics =
        super::CallArgumentDiagnosticContext::from_syntax(super::CallArgumentSyntax::Supported {
            callee_name: None,
        });
    let receiving_context = super::CallArgumentReceivingContext::with_policies(
        diagnostics,
        super::CallArgumentNamingPolicy::NamedOnly,
        super::CallArgumentValuePolicy::ConstRequired,
    );
    let args = parse_args_with_receiving_context("record(value = 1 + 2)", receiving_context, None)
        .expect("constant arguments should parse");

    assert_eq!(args.len(), 1);
    assert!(args[0].target_param.is_some());
    assert!(args[0].parameter_slot.is_none());
}

#[test]
fn const_required_receiving_context_rejects_non_constant_values() {
    let diagnostics =
        super::CallArgumentDiagnosticContext::from_syntax(super::CallArgumentSyntax::Supported {
            callee_name: None,
        });
    let receiving_context = super::CallArgumentReceivingContext::with_policies(
        diagnostics,
        super::CallArgumentNamingPolicy::NamedOnly,
        super::CallArgumentValuePolicy::ConstRequired,
    );
    let error =
        parse_args_with_receiving_context("record(value = {1 = 2})", receiving_context, None)
            .expect_err("const-required arguments must reject map literals");

    let ExpressionParseError::Diagnostic(diagnostic) = error else {
        panic!("expected a user diagnostic for a non-constant argument");
    };
    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    let facts = diagnostic
        .family_view::<CompileTimeEvaluationErrorView>()
        .expect("the diagnostic is a compile-time evaluation failure");
    assert_eq!(
        facts.reason,
        CompileTimeEvaluationErrorReason::ConstantInitializerNotFoldable
    );
}

#[test]
fn retains_parser_selected_parameter_slots_for_named_and_positional_arguments() {
    let args = parse_args_with_parameter_names("call(1, c = 3, b = 2)", &["a", "b", "c"]);

    assert_eq!(args.len(), 3);
    assert_eq!(args[0].parameter_slot.map(|slot| slot.index()), Some(0));
    assert_eq!(args[1].parameter_slot.map(|slot| slot.index()), Some(2));
    assert_eq!(args[2].parameter_slot.map(|slot| slot.index()), Some(1));
}

#[test]
fn final_validation_consumes_retained_slots_for_defaults_and_access_policy() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let file_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let mut span_builder = ExtendedSpanBuilder::new();
    let tokens = tokenize(
        "call(1, third = 3)",
        file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        &mut path_fork,
        SourceId::COMPILATION_ROOT,
        &mut span_builder,
    )
    .expect("tokenization should succeed");
    let range = tokens
        .tokens
        .full_range()
        .expect("test token stream must expose a checked full range");
    let mut token_stream = AstCursor::from_source_tokens(&tokens.tokens, range)
        .expect("test token stream must expose an AST cursor");

    while token_stream.current_tag() != TokenTag::OPEN_PARENTHESIS {
        token_stream.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut expression_builder =
        crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder::new()
            .unwrap();
    let mut const_values =
        crate::compiler_frontend::ast::const_values::store::ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let i32_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let expectations = [
        ParameterExpectation {
            name: Some(string_table.intern("first")),
            expected_type: ExpectedParameterType::Known(i32_type_id),
            access_mode: ExpectedAccessMode::Mutable,
            default_value: None,
            contract: ParameterContract::Value,
        },
        ParameterExpectation {
            name: Some(string_table.intern("second")),
            expected_type: ExpectedParameterType::Known(i32_type_id),
            access_mode: ExpectedAccessMode::Shared,
            default_value: Some(ParameterDefault::Folded(const_values.insert_resolved(i32_type_id, crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 is an I32 value"),
            ), None).expect("fixture default"))),
            contract: ParameterContract::Value,
        },
        ParameterExpectation {
            name: Some(string_table.intern("third")),
            expected_type: ExpectedParameterType::Known(i32_type_id),
            access_mode: ExpectedAccessMode::Shared,
            default_value: None,
            contract: ParameterContract::Value,
        },
    ];

    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let arguments = super::parse_call_arguments_typed_with_expectations(
        &mut token_stream,
        &context,
        &mut type_interner,
        &mut expression_builder,
        &mut const_values,
        &mut string_table,
        &expectations,
        super::CallArgumentSyntax::Supported { callee_name: None },
        &mut path_fork,
    )
    .expect("call arguments should parse");

    let span = arguments[0].span;
    let type_check_context = type_interner.type_check_context();
    let resolved = resolve_call_arguments(
        CallDiagnosticContext::function(CallDiagnosticName::Borrowed("call")),
        arguments,
        &expectations,
        span,
        CallArgumentResolutionContext {
            expression_builder: &mut expression_builder,
            const_values: &mut const_values,
            string_table: &mut string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
            path_fork: &path_fork,
        },
    )
    .unwrap_or_else(|_| panic!("retained slots should resolve without rerouting"));

    assert!(matches!(
        expression_builder.view_owned(resolved[0].value).expect("owned argument").operation(),
        ExpressionOperationView::Constant(id)
            if ast_const_payload_is_fixed_scalar(&const_values, id, FixedScalarValue::signed(FixedScalar::I32, 1).expect("1 is an I32 value"))
    ));
    assert!(matches!(
        expression_builder.view_owned(resolved[1].value).expect("owned argument").operation(),
        ExpressionOperationView::Constant(id)
            if ast_const_payload_is_fixed_scalar(&const_values, id, FixedScalarValue::signed(FixedScalar::I32, 2).expect("2 is an I32 value"))
    ));
    assert!(matches!(
        expression_builder.view_owned(resolved[2].value).expect("owned argument").operation(),
        ExpressionOperationView::Constant(id)
            if ast_const_payload_is_fixed_scalar(&const_values, id, FixedScalarValue::signed(FixedScalar::I32, 3).expect("3 is an I32 value"))
    ));
    assert_eq!(resolved[0].passing_mode, CallPassingMode::FreshMutableValue);
}

#[test]
fn optional_call_context_is_limited_to_bare_none_arguments() {
    let _ = parse_single_file_ast(
        "consume |first String?, second String?|:\n    io.line(\"ok\")\n;\n\nconsume(\n    none\n    ,\n    none\n)\n",
    );

    let failure = parse_single_file_ast_diagnostic(
        "consume |message String?|:\n    io.line(\"ok\")\n;\n\nmaybe Bool? = none\nconsume(none is maybe)\n",
    );
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-TYPE-0001", None);
    let facts = diagnostic
        .family_view::<TypeMismatchView>()
        .expect("function-argument type mismatch should retain typed facts");
    assert_eq!(facts.context, TypeMismatchContext::FunctionArgument);
}

#[test]
fn whole_numeric_call_arguments_use_f64_parameter_destinations() {
    let expectation = [ParameterExpectation {
        name: None,
        expected_type: ExpectedParameterType::Known(builtin_type_ids::fixed_scalar(
            FixedScalar::F64,
        )),
        access_mode: ExpectedAccessMode::Shared,
        default_value: None,
        contract: ParameterContract::Value,
    }];
    let (arguments, expression_builder, const_values) = parse_owned_args_with_receiving_context(
        "identity(18_446_744_073_709_551_615)",
        super::CallArgumentReceivingContext::existing_call(super::CallArgumentSyntax::Supported {
            callee_name: None,
        }),
        Some(&expectation),
    )
    .expect("an F64 parameter should receive an out-of-I32-range whole literal");
    let expected = FixedScalarValue::binary_float(FixedScalar::F64, 18_446_744_073_709_551_615_f64)
        .expect("exact F64 identity");
    assert_eq!(
        expression_builder
            .view_owned(arguments[0].value)
            .expect("owned argument")
            .require_single(
                crate::compiler_frontend::compiler_messages::ValueReceiverContext::FunctionArgument
            )
            .expect("single value"),
        builtin_type_ids::fixed_scalar(FixedScalar::F64)
    );
    assert!(
        matches!(
            expression_builder.view_owned(arguments[0].value).expect("owned argument").operation(),
            ExpressionOperationView::Constant(id)
            if ast_const_payload_is_fixed_scalar(&const_values, id, expected)
        ),
        "expected the call argument to retain exact F64 identity"
    );
}

#[test]
fn generic_call_diagnostic_retains_exact_multibyte_callee_span() {
    let function_name = "f".repeat(1024);
    let source = format!(
        "{function_name} type T || -> {{T}}:\n    return {{}}\n;\n\nprefix = \"é\"\nvalue = {function_name}()\n"
    );
    let failure = parse_single_file_ast_diagnostic(&source);
    let call_start = source
        .rfind(&format!("{function_name}()"))
        .expect("the generic call should be present") as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    // The declaration's long name occupies the first extended row in the tokenizer.
    let _declaration_span =
        LocalSpan::exact(0, function_name.len() as u32, &mut span_builder).unwrap();
    let call_span = LocalSpan::exact(call_start, function_name.len() as u32, &mut span_builder)
        .expect("the callee should fit the extended span table");

    assert_eq!(span_builder.len(), 2);
    assert_eq!(
        ast_error_primary_span(first_ast_error_view(&failure)),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, call_span))
    );
}

#[test]
fn rejects_mutable_marker_on_named_argument_target() {
    let failure = parse_single_file_ast_diagnostic(
        r#"
take |value ~I32|:
;

value ~= 1
take(~value = value)
"#,
    );
    assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0002", None);
}

#[test]
fn rejects_tilde_on_left_side_of_named_arg() {
    // ~name = value is explicitly rejected at the parse level before signature binding.
    let diagnostic = parse_args_diagnostic("take(~value = 1)");

    let compact = compact_bag_for_test(DiagnosticBag::from(diagnostic), None);
    let diagnostic = compact.first().expect("the diagnostic is retained");
    assert_eq!(diagnostic.descriptor().code, "MOTH-SYNTAX-0002");
    let facts = diagnostic
        .family_view::<UnexpectedTokenView>()
        .expect("the diagnostic is unexpected-token");
    assert!(matches!(
        facts.shape,
        UnexpectedTokenShape::Token(found)
            if found == DiagnosticToken::from_static_tag(TokenTag::MUTABLE)
    ));
}

// ── Source-span diagnostics for mutable-access call shape ───────────────────

#[test]
fn mutable_marker_on_immutable_argument_uses_authored_marker_span() {
    let source = r#"
mutate |value ~I32|:
    ~value = 5
;

x = 1
mutate(~x)
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0054",
        Some("invalid_call_shape.mutable_access_on_immutable_place"),
    );
    let marker_start = source.rfind("~x").unwrap() as u32;
    let marker_span = LocalSpan::exact(marker_start, 1, &mut ExtendedSpanBuilder::new()).unwrap();
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, marker_span))
    );
}

#[test]
fn mutable_marker_on_fresh_value_uses_authored_marker_span() {
    for argument in ["~12", "~copy source"] {
        let source =
            format!("mutate |value ~I32|:\n    ~value = 5\n;\nsource ~= 1\nmutate({argument})\n");
        let failure = parse_single_file_ast_diagnostic(&source);
        let diagnostic = assert_ast_diagnostic(
            &failure,
            "MOTH-RULE-0054",
            Some("invalid_call_shape.mutable_access_on_non_place"),
        );
        let marker_start = source.rfind(argument).unwrap() as u32;
        let marker_span =
            LocalSpan::exact(marker_start, 1, &mut ExtendedSpanBuilder::new()).unwrap();
        assert_eq!(
            ast_error_primary_span(diagnostic),
            Some(SourceSpan::new(SourceId::COMPILATION_ROOT, marker_span))
        );
    }
}

#[test]
fn unmarked_immutable_argument_uses_value_expression_span() {
    let source = r#"
mutate |value ~I32|:
    ~value = 5
;

x = 1
mutate(x)
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0054",
        Some("invalid_call_shape.immutable_place_mutable_access_required"),
    );
    let value_start = source.rfind("mutate(x)").unwrap() as u32 + "mutate(".len() as u32;
    let value_span = LocalSpan::exact(value_start, 1, &mut ExtendedSpanBuilder::new()).unwrap();
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, value_span))
    );
}

#[test]
fn projected_immutable_argument_keeps_root_and_generic_repair() {
    for (argument, reason) in [
        (
            "box.value",
            "invalid_call_shape.immutable_place_mutable_access_required",
        ),
        (
            "~box.value",
            "invalid_call_shape.mutable_access_on_immutable_place",
        ),
    ] {
        let source = format!(
            "Box = |\n    value I32,\n|\n\nmutate |value ~I32|:\n    ~value = 5\n;\n\nbox = Box(1)\nmutate({argument})\n"
        );
        let failure = parse_single_file_ast_diagnostic(&source);
        assert_ast_diagnostic(&failure, "MOTH-RULE-0054", Some(reason));
        let message = ast_first_error_rendered_message(&failure);
        assert!(message.contains("`box` is immutable"), "{message}");
        assert!(
            message.contains("pass the existing argument with `~`"),
            "{message}"
        );
        assert!(!message.contains("`~box`"), "{message}");
    }
}

#[test]
fn projected_mutable_receiver_missing_marker_uses_generic_repair() {
    let source = "Counter = |\n    value I32,\n|\n\nHolder = |\n    counter Counter,\n|\n\nreset |this ~Counter|:\n    ~this.value = 0\n;\n\nholder ~= Holder(Counter(1))\nholder.counter.reset()\n";
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0047",
        Some("invalid_receiver_call.mutable_receiver_missing_marker"),
    );
    let message = ast_first_error_rendered_message(&failure);
    assert!(
        message.contains("Prefix the receiver with `~`."),
        "{message}"
    );
    assert!(!message.contains("`~holder.reset(...)`"), "{message}");
    assert!(!message.contains("for example"), "{message}");
}

#[test]
fn mutable_marker_on_shared_parameter_is_rejected_at_authored_marker() {
    let source = r#"
inspect |value I32|:
;

x ~= 1
inspect(~x)
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0054",
        Some("invalid_call_shape.mutable_access_not_allowed"),
    );
    let marker_start = source.rfind("~x").unwrap() as u32;
    let marker_span = LocalSpan::exact(marker_start, 1, &mut ExtendedSpanBuilder::new()).unwrap();
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, marker_span))
    );
}
