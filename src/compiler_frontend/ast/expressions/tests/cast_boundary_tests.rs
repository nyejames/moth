//! Cast evidence and boundary fact tests.
//!
//! WHAT: checks resolved builtin cast evidence, fallibility, delivery and failure facts at
//!       parser-owned call/receiver boundaries, along with targeted invalid diagnostics.
//! WHY: casts must retain their concrete receiving target and delivery facts after AST
//!      construction; generic-bound evidence consumption and source acceptance for concrete call
//!      and constructor boundaries belong to the end-to-end fixture suite.

use crate::compiler_frontend::ast::Ast;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::ConstValuePayload;
use crate::compiler_frontend::ast::expressions::expression_store::{
    CastFacts, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    CastHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::failure_facts::ImplicitFailureSource;
use crate::compiler_frontend::builtins::casts::targets::{
    BuiltinCastFallibility, BuiltinCastPolicyId,
};
use crate::compiler_frontend::compiler_messages::InvalidCastReason;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_body_by_name, function_signature_by_name, start_function_body,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_primary_span, parse_single_file_ast,
    parse_single_file_ast_diagnostic,
};
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};

fn assert_invalid_cast(source: &str, expected_reason: InvalidCastReason) {
    let reason_key = match expected_reason {
        InvalidCastReason::MissingExplicitTarget => "invalid_cast.missing_explicit_target",
        InvalidCastReason::TargetNotBuiltin => "invalid_cast.target_not_builtin",
        InvalidCastReason::TargetIsGenericParameter => "invalid_cast.target_is_generic_parameter",
        InvalidCastReason::SameSourceAndTarget => "invalid_cast.same_source_and_target",
        InvalidCastReason::SourceIsOptional => "invalid_cast.source_is_optional",
        InvalidCastReason::OperandArityMismatch => "invalid_cast.operand_arity_mismatch",
        InvalidCastReason::TargetArityMismatch => "invalid_cast.target_arity_mismatch",
        InvalidCastReason::CastPropagationRemoved => "invalid_cast.cast_propagation_removed",
        InvalidCastReason::NoEvidence => "invalid_cast.no_evidence",
        InvalidCastReason::BuiltinEvidenceNotConstFoldable => {
            "invalid_cast.builtin_evidence_not_const_foldable"
        }
        InvalidCastReason::UserDefinedEvidenceNotConstFoldable => {
            "invalid_cast.user_defined_evidence_not_const_foldable"
        }
        InvalidCastReason::GenericBoundEvidenceNotConstFoldable => {
            "invalid_cast.generic_bound_evidence_not_const_foldable"
        }
        InvalidCastReason::BuiltinCastFailedInConst => "invalid_cast.builtin_cast_failed_in_const",
    };
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-RULE-0083", Some(reason_key));
}

fn assert_invalid_call_shape(source: &str, expected_reason_key: &'static str) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-RULE-0054", Some(expected_reason_key));
}

fn returned_expression<'a>(
    ast: &'a Ast,
    path_fork: &PathInternerFork,
    string_table: &StringTable,
    function_name: &str,
) -> ExpressionView<'a> {
    let body = function_body_by_name(ast, path_fork, string_table, function_name);
    let id = body
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Return(values) => values.first(),
            _ => None,
        })
        .expect("function should return a value");
    ast.expressions
        .view(*id)
        .expect("returned expression belongs to AST")
}

fn resolved_cast<'a>(expression: ExpressionView<'a>) -> &'a CastFacts {
    let ExpressionOperationView::Cast { facts, .. } = expression.operation() else {
        panic!("expected a resolved cast expression");
    };
    facts
}

// ------------------------
//  Source function parameters
// ------------------------

#[test]
fn unknown_named_parameter_with_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x I32, y I32| -> I32:
    return x + y
;

value = draw(x = 0, missing = cast "2")
"#,
        "invalid_call_shape.named_argument_not_found",
    );
}

#[test]
fn duplicate_named_parameter_with_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x I32, y I32| -> I32:
    return x + y
;

value = draw(x = 0, x = cast "2")
"#,
        "invalid_call_shape.duplicate_argument",
    );
}

#[test]
fn duplicate_named_parameter_preserves_exact_target_span() {
    let source = r#"
draw |x I32, y I32| -> I32:
    return x + y
;

value = draw(x = 0, x = cast "2")
"#;
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0054",
        Some("invalid_call_shape.duplicate_argument"),
    );

    let target_start = source
        .rfind("x = cast \"2\"")
        .expect("the duplicate named argument should be present") as u32;
    let mut span_builder = ExtendedSpanBuilder::new();
    let target_span = LocalSpan::exact(target_start, 1, &mut span_builder)
        .expect("the duplicate target span should fit inline");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, target_span))
    );
}

#[test]
fn positional_after_named_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x I32, y I32| -> I32:
    return x + y
;

value = draw(x = 0, cast "2")
"#,
        "invalid_call_shape.positional_after_named",
    );
}

#[test]
fn extra_positional_cast_reports_call_shape() {
    assert_invalid_call_shape(
        r#"
draw |x I32| -> I32:
    return x
;

value = draw(0, cast "2")
"#,
        "invalid_call_shape.extra_positional_argument",
    );
}

#[test]
fn generic_function_parameter_rejects_cast_target() {
    assert_invalid_cast(
        r#"
identity type T |value T| -> T:
    return value
;

value I32 = identity(cast "1")
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
    price I32,
|

add |this Item, amount I32| -> Item:
    return Item(price = this.price + amount)
;

apply_text |text String| -> Item:
    return Item(price = 1).add(amount = cast text)
;
"#,
    );
    let expression = returned_expression(&ast, &path_fork, &string_table, "apply_text");
    let ExpressionOperationView::MethodCall { call, .. } = expression.operation() else {
        panic!("expected a receiver method call");
    };
    assert_eq!(
        call.arguments().len(),
        1,
        "expected one receiver method argument"
    );
    let argument = call.arguments().iter().next().expect("one argument");
    let cast = resolved_cast(argument.value());

    assert_eq!(cast.fallibility, BuiltinCastFallibility::Fallible);
    assert!(matches!(
        &cast.evidence,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32))
        }
    ));
    assert!(matches!(&cast.handling, CastHandling::Implicit));
    assert!(
        argument
            .value()
            .failure_facts()
            .expect("cast failure facts")
            .authored_cast
    );
    assert!(matches!(
        argument.value().failure_facts().expect("cast failure facts").implicit.as_slice(),
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
    count I32 = cast "not an integer",
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
parse_count |text String| -> I32, Error!:
    return cast text
;
"#;
    let (ast, path_fork, string_table) = parse_single_file_ast(source);
    let signature = function_signature_by_name(&ast, &path_fork, &string_table, "parse_count");
    let expression = returned_expression(&ast, &path_fork, &string_table, "parse_count");
    let cast = resolved_cast(expression);

    assert!(signature.error_return_type_id().is_some());
    assert_eq!(cast.fallibility, BuiltinCastFallibility::Fallible);
    assert!(matches!(
        &cast.evidence,
        ResolvedCastEvidence::Builtin {
            policy: BuiltinCastPolicyId::StringToNumeric(NumericScalar::Fixed(FixedScalar::I32))
        }
    ));
    assert!(matches!(&cast.handling, CastHandling::Implicit));
    assert!(matches!(
        expression.failure_facts().expect("cast failure facts").implicit.as_slice(),
        [contributor] if contributor.source == ImplicitFailureSource::AuthoredCastConversion
    ));
}

#[test]
fn concrete_fallible_cast_rejects_removed_bang_spelling() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> I32, Error!:
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
value I32 = cast "42" catch:
    then 0
;
"#,
    );
    let body = start_function_body(&ast, &path_fork, &string_table);
    let NodeKind::VariableDeclaration(declaration) = &body[0].kind else {
        panic!("expected the constant initializer");
    };

    assert!(matches!(
        ast.expressions.view(declaration.value.initializer().expect("fixture initializer")).expect("owned initializer").operation(),
        ExpressionOperationView::Constant(id)
            if matches!(ast.const_values.payload(id), Some(ConstValuePayload::FixedScalar(value))
            if *value == FixedScalarValue::signed(FixedScalar::I32, 42)
                .expect("42 is an I32 value"))
    ));
}

#[test]
fn concrete_fallible_cast_rejects_removed_bang_even_with_recovery() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> I32, Error!:
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
    let failure = parse_single_file_ast_diagnostic(
        r#"
values ~= {1}
~values.push(value = cast "2")
"#,
    );
    assert_ast_diagnostic(
        &failure,
        "MOTH-RULE-0046",
        Some("invalid_builtin_call.named_arguments_not_supported"),
    );
}
