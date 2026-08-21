//! Function call argument parser and source-location regression tests.
//!
//! WHAT: protects raw parsed argument shape, call-access mode classification, and the distinct
//!       named-target, value-expression and authored-marker source locations produced by the
//!       call argument parser.
//! WHY: parser-local facts and the parser-to-final-validation retained-slot handoff are internal
//!      invariants that end-to-end integration output cannot inspect. Whole-source call acceptance
//!      and rejection behavior is owned by canonical integration cases under
//!      `tests/cases/function_call_*`.

use crate::compiler_frontend::ast::expressions::call_argument::{
    CallAccessMode, CallArgument, CallPassingMode,
};
use crate::compiler_frontend::ast::expressions::call_validation::{
    CallArgumentResolutionContext, CallDiagnosticContext, ExpectedAccessMode,
    ExpectedParameterType, ParameterExpectation, resolve_call_arguments,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, InvalidCallShapeReason,
    SyntaxDiagnosticKind, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind, TokenizerEntryMode};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::value_mode::ValueMode;
use std::rc::Rc;
use std::sync::Arc;

fn parse_args(
    source: &str,
) -> Vec<crate::compiler_frontend::ast::expressions::call_argument::CallArgument> {
    let mut string_table = StringTable::new();
    let file_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let mut tokens = tokenize(
        source,
        &file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    while tokens.current_token_kind() != &TokenKind::OpenParenthesis {
        tokens.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    parse_raw_call_args_for_test(&mut tokens, &context, &mut type_interner, &mut string_table)
        .expect("call arguments should parse")
}

fn parse_args_with_parameter_names(source: &str, parameter_names: &[&str]) -> Vec<CallArgument> {
    let mut string_table = StringTable::new();
    let file_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let mut tokens = tokenize(
        source,
        &file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    while tokens.current_token_kind() != &TokenKind::OpenParenthesis {
        tokens.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let expectations = parameter_names
        .iter()
        .map(|name| ParameterExpectation {
            name: Some(string_table.intern(name)),
            expected_type: ExpectedParameterType::UnknownExternal,
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: None,
        })
        .collect::<Vec<_>>();

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    super::parse_call_arguments_typed_with_expectations(
        &mut tokens,
        &context,
        &mut type_interner,
        &mut string_table,
        &expectations,
        super::NamedArgumentSyntax::Supported { callee_name: None },
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
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
) -> Result<Vec<CallArgument>, ExpressionParseError> {
    super::parse_call_arguments_inner(
        token_stream,
        context,
        type_interner,
        string_table,
        super::CallArgumentSyntaxContext::Ordinary,
        super::NamedArgumentSyntax::Supported { callee_name: None },
        None,
    )
}

fn parse_args_diagnostic(source: &str) -> CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let file_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let mut tokens = tokenize(
        source,
        &file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    while tokens.current_token_kind() != &TokenKind::OpenParenthesis {
        tokens.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let error =
        parse_raw_call_args_for_test(&mut tokens, &context, &mut type_interner, &mut string_table)
            .expect_err("call arguments should fail");

    CompilerDiagnostic::from(error)
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
fn call_argument_locations_keep_named_target_value_and_marker_distinct() {
    // `parameter = ~value` must keep the named-target token, the value expression and the
    // authored `~` marker at three distinct source locations so diagnostics can label the
    // source the author must change. The value here is a literal, not a binding.
    let args = parse_args("take(value = ~1)");

    assert_eq!(args.len(), 1);
    let argument = &args[0];
    assert_eq!(argument.access_mode, CallAccessMode::Mutable);

    let target_location = argument
        .target_location
        .clone()
        .expect("named argument should carry a target location");
    let marker_location = argument
        .marker_location
        .clone()
        .expect("authored ~ should carry a marker location");

    // `location` is the value-expression location, not the named-target token.
    assert_ne!(argument.location, target_location);
    assert_ne!(argument.location, marker_location);
    assert_ne!(target_location, marker_location);
}

#[test]
fn call_argument_marker_location_is_absent_without_authored_tilde() {
    let args = parse_args("take(value = 1)");

    assert_eq!(args.len(), 1);
    assert_eq!(args[0].access_mode, CallAccessMode::Shared);
    assert!(
        args[0].marker_location.is_none(),
        "absent ~ must not synthesize a marker location",
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
    let file_path = InternedPath::from_single_str("@page.moth", &mut string_table);
    let mut tokens = tokenize(
        "call(1, third = 3)",
        &file_path,
        TokenizerEntryMode::SourceFile,
        &crate::compiler_frontend::style_directives::StyleDirectiveRegistry::built_ins(),
        &mut string_table,
        None,
    )
    .expect("tokenization should succeed");

    while tokens.current_token_kind() != &TokenKind::OpenParenthesis {
        tokens.advance();
    }

    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        InternedPath::new(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut type_environment = TypeEnvironment::new();
    let int_type_id = type_environment.builtins().int;
    let expectations = [
        ParameterExpectation {
            name: Some(string_table.intern("first")),
            expected_type: ExpectedParameterType::Known(int_type_id),
            access_mode: ExpectedAccessMode::Mutable,
            requires_reactive_source: false,
            default_value: None,
        },
        ParameterExpectation {
            name: Some(string_table.intern("second")),
            expected_type: ExpectedParameterType::Known(int_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: Some(Expression::int(
                2,
                Default::default(),
                ValueMode::ImmutableOwned,
            )),
        },
        ParameterExpectation {
            name: Some(string_table.intern("third")),
            expected_type: ExpectedParameterType::Known(int_type_id),
            access_mode: ExpectedAccessMode::Shared,
            requires_reactive_source: false,
            default_value: None,
        },
    ];

    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
    let arguments = super::parse_call_arguments_typed_with_expectations(
        &mut tokens,
        &context,
        &mut type_interner,
        &mut string_table,
        &expectations,
        super::NamedArgumentSyntax::Supported { callee_name: None },
    )
    .expect("call arguments should parse");

    let location = arguments[0].location.clone();
    let type_check_context = type_interner.type_check_context();
    let resolved = resolve_call_arguments(
        CallDiagnosticContext::function("call"),
        &arguments,
        &expectations,
        location,
        CallArgumentResolutionContext {
            string_table: &mut string_table,
            type_environment: type_check_context.type_environment,
            compatibility_cache: type_check_context.compatibility_cache,
        },
    )
    .unwrap_or_else(|_| panic!("retained slots should resolve without rerouting"));

    assert!(matches!(resolved[0].value.kind, ExpressionKind::Int(1)));
    assert!(matches!(resolved[1].value.kind, ExpressionKind::Int(2)));
    assert!(matches!(resolved[2].value.kind, ExpressionKind::Int(3)));
    assert_eq!(resolved[0].passing_mode, CallPassingMode::FreshMutableValue);
}

#[test]
fn optional_call_context_is_limited_to_bare_none_arguments() {
    let _ = parse_single_file_ast(
        "consume |first String?, second String?|:\n    io.line(\"ok\")\n;\n\nconsume(\n    none\n    ,\n    none\n)\n",
    );

    let diagnostic = parse_single_file_ast_diagnostic(
        "consume |message String?|:\n    io.line(\"ok\")\n;\n\nmaybe Bool? = none\nconsume(none is maybe)\n",
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::TypeMismatch {
            context: TypeMismatchContext::FunctionArgument,
            ..
        }
    ));
}

#[test]
fn rejects_mutable_marker_on_named_argument_target() {
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
take |value ~Int|:
;

value ~= 1
take(~value = value)
"#,
    );

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken { .. }
    ));
}

#[test]
fn rejects_tilde_on_left_side_of_named_arg() {
    // ~name = value is explicitly rejected at the parse level before signature binding.
    let diagnostic = parse_args_diagnostic("take(~value = 1)");

    assert_eq!(
        diagnostic.kind,
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken)
    );
    assert_eq!(diagnostic.kind.code(), "MOTH-SYNTAX-0002");
    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::UnexpectedToken {
            found: TokenKind::Mutable
        }
    ));
}

// ── Source-location diagnostics for mutable-access call shape ────────────────

#[test]
fn mutable_marker_on_immutable_argument_uses_authored_marker_location() {
    // `mutate(~x)` places the authored `~` at column 8 and the value `x` at column 9.
    // The primary label must point at the marker, because the authored `~` is the source the
    // author must remove or repair.
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
mutate |value ~Int|:
    value = 5
;

x = 1
mutate(~x)
"#,
    );

    let DiagnosticPayload::InvalidCallShape { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCallShape diagnostic, got {:?}",
            diagnostic.payload
        )
    };
    assert!(
        matches!(
            reason,
            InvalidCallShapeReason::MutableAccessOnImmutablePlace { .. }
        ),
        "expected MutableAccessOnImmutablePlace, got {reason:?}"
    );

    let location = &diagnostic.primary_location;
    assert_eq!(
        location.start_pos.line_number, 6,
        "marker is on the call line"
    );
    assert_eq!(
        location.start_pos.char_column, 8,
        "marker `~` sits at column 8"
    );
    assert_ne!(
        location.start_pos.char_column, 9,
        "must not point at the value `x` at column 9"
    );
}

#[test]
fn mutable_marker_on_fresh_value_uses_authored_marker_location() {
    // `mutate(~12)` places the authored `~` at column 8 and the fresh literal at column 9.
    // The primary label must point at the marker, since the plain fresh value is valid and the
    // authored `~` is the mistake.
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
mutate |value ~Int|:
    value = 5
;

mutate(~12)
"#,
    );

    let DiagnosticPayload::InvalidCallShape { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCallShape diagnostic, got {:?}",
            diagnostic.payload
        )
    };
    assert!(
        matches!(
            reason,
            InvalidCallShapeReason::MutableAccessOnNonPlace { .. }
        ),
        "expected MutableAccessOnNonPlace, got {reason:?}"
    );

    let location = &diagnostic.primary_location;
    assert_eq!(
        location.start_pos.line_number, 5,
        "marker is on the call line"
    );
    assert_eq!(
        location.start_pos.char_column, 8,
        "marker `~` sits at column 8"
    );
    assert_ne!(
        location.start_pos.char_column, 9,
        "must not point at the fresh value at column 9"
    );
}

#[test]
fn unmarked_immutable_argument_uses_value_expression_location() {
    // `mutate(x)` has no authored `~`, so the value expression `x` is the call-site source the
    // author must change. The primary label must point at the value, at column 8.
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
mutate |value ~Int|:
    value = 5
;

x = 1
mutate(x)
"#,
    );

    let DiagnosticPayload::InvalidCallShape { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCallShape diagnostic, got {:?}",
            diagnostic.payload
        )
    };
    assert!(
        matches!(
            reason,
            InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired { .. }
        ),
        "expected ImmutablePlaceMutableAccessRequired, got {reason:?}"
    );

    let location = &diagnostic.primary_location;
    assert_eq!(
        location.start_pos.line_number, 6,
        "value is on the call line"
    );
    assert_eq!(
        location.start_pos.char_column, 8,
        "value expression `x` sits at column 8"
    );
}
