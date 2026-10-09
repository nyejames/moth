//! Cast evidence and boundary fact tests.
//!
//! WHAT: checks resolved builtin cast evidence, fallibility, delivery and failure facts at
//!       parser-owned call/receiver boundaries, along with targeted invalid diagnostics.
//! WHY: casts must retain their concrete receiving target and delivery facts after AST
//!      construction; generic-bound evidence consumption and source acceptance for concrete call
//!      and constructor boundaries belong to the end-to-end fixture suite.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_kind::ResolvedCastExpression;
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureSource;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId, BuiltinCastTarget,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidBuiltinCallReason, InvalidCallShapeReason, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_signature_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    parse_single_file_ast, parse_single_file_ast_diagnostic,
};

fn assert_invalid_cast(source: &str, expected_reason: InvalidCastReason) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    let DiagnosticPayload::InvalidCast { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCast diagnostic, got {:?}",
            diagnostic.payload
        );
    };

    assert_eq!(
        *reason, expected_reason,
        "unexpected InvalidCast reason for source:\n{source}"
    );
}

fn assert_invalid_call_shape(
    source: &str,
    reason_matches: impl FnOnce(&InvalidCallShapeReason) -> bool,
) {
    let diagnostic = parse_single_file_ast_diagnostic(source);

    let DiagnosticPayload::InvalidCallShape { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCallShape diagnostic, got {:?}",
            diagnostic.payload
        );
    };

    assert!(reason_matches(reason));
}

fn returned_expression<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function_name: &str,
) -> &'a Expression {
    let body = function_body_by_name(ast, path_fork, string_table, function_name);
    body.iter()
        .find_map(|node| match &node.kind {
            NodeKind::Return(values) => values.first(),
            _ => None,
        })
        .expect("function should return a value")
}

fn resolved_cast(expression: &Expression) -> &ResolvedCastExpression {
    let ExpressionKind::Cast(cast) = &expression.kind else {
        panic!(
            "expected a resolved cast expression, found {:?}",
            expression.kind
        );
    };
    cast
}

// ------------------------
//  Source function parameters
// ------------------------

#[test]
fn unknown_named_parameter_with_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(x = 0, missing = cast "2")
"#,
        |reason| matches!(reason, InvalidCallShapeReason::NamedArgumentNotFound { .. }),
    );
}

#[test]
fn duplicate_named_parameter_with_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(x = 0, x = cast "2")
"#,
        |reason| matches!(reason, InvalidCallShapeReason::DuplicateArgument { .. }),
    );
}

#[test]
fn duplicate_named_parameter_preserves_exact_target_span() {
    let source = r#"
draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(x = 0, x = cast "2")
"#;
    let diagnostic = parse_single_file_ast_diagnostic(source);

    let DiagnosticPayload::InvalidCallShape { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidCallShape diagnostic, got {:?}",
            diagnostic.payload
        );
    };
    assert!(matches!(
        reason,
        InvalidCallShapeReason::DuplicateArgument { .. }
    ));

    let target_start = source
        .rfind("x = cast \"2\"")
        .expect("the duplicate named argument should be present") as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    let target_span = LocalSpan::exact(target_start, 1, &mut span_builder)
        .expect("the duplicate target span should fit inline");
    assert_eq!(
        diagnostic.primary_span,
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, target_span))
    );
}

#[test]
fn positional_after_named_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(x = 0, cast "2")
"#,
        |reason| matches!(reason, InvalidCallShapeReason::PositionalAfterNamed),
    );
}

#[test]
fn extra_positional_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x Int| -> Int:
    return x
;

value = draw(0, cast "2")
"#,
        |reason| {
            matches!(
                reason,
                InvalidCallShapeReason::ExtraPositionalArgument { .. }
            )
        },
    );
}

#[test]
fn generic_function_parameter_rejects_cast_target() {
    assert_invalid_cast(
        r#"
identity type T |value T| -> T:
    return value
;

value Int = identity(cast "1")
"#,
        InvalidCastReason::TargetIsGenericParameter,
    );
}

// ------------------------
//  Struct constructors
// ------------------------

#[test]
fn generic_struct_constructor_field_rejects_cast_target() {
    assert_invalid_cast(
        r#"
Box type A = |
    value A,
|

value = Box(value = cast "1")
"#,
        InvalidCastReason::TargetIsGenericParameter,
    );
}

// ------------------------
//  Receiver method parameters
// ------------------------

#[test]
fn receiver_method_parameter_records_plain_fallible_cast_facts() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        r#"
Item = |
    price Int,
|

add |this Item, amount Int| -> Item:
    return Item(price = this.price + amount)
;

apply_text |text String| -> Item:
    return Item(price = 1).add(amount = cast text)
;
"#,
    );
    let expression = returned_expression(&ast, &path_fork, &string_table, "apply_text");
    let ExpressionKind::MethodCall { args, .. } = &expression.kind else {
        panic!("expected a receiver method call");
    };
    let [argument] = args.as_slice() else {
        panic!("expected one receiver method argument");
    };
    let cast = resolved_cast(&argument.value);

    assert_eq!(cast.target, BuiltinCastTarget::Int);
    assert_eq!(cast.fallibility, BuiltinCastFallibility::Fallible);
    assert!(matches!(
        &cast.evidence,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int)
        }
    ));
    assert!(matches!(&cast.handling, CastHandling::Implicit));
    assert!(argument.value.failure_facts.authored_cast);
    assert!(matches!(
        argument.value.failure_facts.implicit.as_slice(),
        [contributor]
            if contributor.source == ImplicitFailureSource::AuthoredCastConversion
                && !contributor.codes.is_empty()
    ));
}

// ------------------------
//  Struct field defaults
// ------------------------

#[test]
fn struct_field_default_keeps_static_cast_failure_diagnostic() {
    assert_invalid_cast(
        r#"
Options = |
    count Int = cast "not an integer",
|

value = Options()
"#,
        InvalidCastReason::BuiltinCastFailedInConst,
    );
}

// ------------------------
//  Generic-bound source evidence
// ------------------------

#[test]
fn generic_function_without_cast_bound_rejects_cast() {
    assert_invalid_cast(
        r#"
render type T |value T| -> String:
    return cast value
;
"#,
        InvalidCastReason::NoEvidence,
    );
}

#[test]
fn generic_function_with_wrong_cast_bound_rejects_cast() {
    assert_invalid_cast(
        r#"
render type T is CASTABLE_TO_I32 |value T| -> String:
    return cast value
;
"#,
        InvalidCastReason::NoEvidence,
    );
}

// ------------------------
//  Cast fallible handling syntax
// ------------------------

#[test]
fn concrete_fallible_cast_records_plain_error_slot_delivery() {
    let source = r#"
parse_count |text String| -> Int, Error!:
    return cast text
;
"#;
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "parse_count");
    let expression = returned_expression(&ast, &path_fork, &string_table, "parse_count");
    let cast = resolved_cast(expression);

    assert!(signature.error_return_type_id().is_some());
    assert_eq!(cast.target, BuiltinCastTarget::Int);
    assert_eq!(cast.fallibility, BuiltinCastFallibility::Fallible);
    assert!(matches!(
        &cast.evidence,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToNumeric(NumericScalar::Int)
        }
    ));
    assert!(matches!(&cast.handling, CastHandling::Implicit));
    assert!(matches!(
        expression.failure_facts.implicit.as_slice(),
        [contributor] if contributor.source == ImplicitFailureSource::AuthoredCastConversion
    ));
}

#[test]
fn concrete_fallible_cast_rejects_removed_bang_spelling() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> Int, Error!:
    return cast ! text
;
"#,
        InvalidCastReason::CastPropagationRemoved,
    );
}

#[test]
fn constant_receiver_keeps_successful_foldable_cast_catch() {
    let (ast, path_fork, string_table) = parse_single_file_ast(
        r#"
value Int = cast "42" catch:
    then 0
;
"#,
    );
    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected the constant initializer");
    };

    assert!(matches!(&declaration.value.kind, ExpressionKind::Int(42)));
}

#[test]
fn concrete_fallible_cast_rejects_removed_bang_even_with_recovery() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> Int, Error!:
    return cast! text catch:
        then 0
    ;
;
"#,
        InvalidCastReason::CastPropagationRemoved,
    );
}

// ------------------------
//  Positional-only builtin parameters
// ------------------------

#[test]
fn builtin_named_parameter_with_cast_reports_builtin_call_shape() {
    let diagnostic = parse_single_file_ast_diagnostic(
        r#"
values ~= {1}
~values.push(value = cast "2")
"#,
    );

    let DiagnosticPayload::InvalidBuiltinCall { reason, .. } = &diagnostic.payload else {
        panic!(
            "expected InvalidBuiltinCall diagnostic, got {:?}",
            diagnostic.payload
        );
    };

    assert_eq!(
        *reason,
        InvalidBuiltinCallReason::NamedArgumentsNotSupported
    );
}
