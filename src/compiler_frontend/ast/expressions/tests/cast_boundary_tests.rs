//! Cast target threading tests for call/constructor boundaries.
//!
//! WHAT: verifies that `cast` receives a concrete builtin target at source/receiver/host
//!      function parameters and struct/choice constructor fields, and that generic parameter
//!      slots reject `cast` with `TargetIsGenericParameter`.
//! WHY: argument parsing owns the cast-target channel at these boundaries; these tests pin the
//!      boundary behavior without depending on backend lowering.

use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidBuiltinCallReason, InvalidCallShapeReason, InvalidCastReason,
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

// ------------------------
//  Source function parameters
// ------------------------

#[test]
fn concrete_function_parameter_accepts_infallible_cast() {
    let _ = parse_single_file_ast(
        r#"
scale |factor Float| -> Float:
    return factor * 2.0
;

value = scale(cast 1)
"#,
    );
}

#[test]
fn concrete_function_parameter_rejects_fallible_cast_without_handling() {
    assert_invalid_cast(
        r#"
draw |x Int, y Int| -> Int:
    return x + y
;

value = draw(cast "1", cast "2")
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

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
fn struct_constructor_field_accepts_infallible_cast() {
    let _ = parse_single_file_ast(
        r#"
Point = |
    x Float,
    y Float,
|

value = Point(x = cast 1, y = cast 2)
"#,
    );
}

#[test]
fn struct_constructor_field_rejects_fallible_cast_without_handling() {
    assert_invalid_cast(
        r#"
Point = |
    x Int,
    y Int,
|

value = Point(x = cast "1", y = 0)
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

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
//  Choice constructors
// ------------------------

#[test]
fn choice_constructor_field_accepts_infallible_cast() {
    let _ = parse_single_file_ast(
        r#"
Measure ::
    Value | amount Float |,
;

value = Measure::Value(amount = cast 1)
"#,
    );
}

#[test]
fn choice_constructor_field_rejects_fallible_cast_without_handling() {
    assert_invalid_cast(
        r#"
Code ::
    Value | code Int |,
;

value = Code::Value(code = cast "1")
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

// ------------------------
//  Receiver method parameters
// ------------------------

#[test]
fn receiver_method_parameter_accepts_infallible_cast() {
    let _ = parse_single_file_ast(
        r#"
Item = |
    price Float,
|

add |this Item, amount Float| -> Item:
    return Item(price = this.price + amount)
;

value = Item(price = 1.0).add(amount = cast 2)
"#,
    );
}

#[test]
fn receiver_method_parameter_rejects_fallible_cast_without_handling() {
    assert_invalid_cast(
        r#"
Item = |
    price Int,
|

add |this Item, amount Int| -> Item:
    return Item(price = this.price + amount)
;

value = Item(price = 1).add(amount = cast "2")
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

// ------------------------
//  Struct field defaults
// ------------------------

#[test]
fn struct_field_default_rejects_fallible_cast_without_handling() {
    assert_invalid_cast(
        r#"
Config = |
    count Int = cast "1",
|

value = Config()
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

// ------------------------
//  Generic-bound source evidence
// ------------------------

#[test]
fn generic_function_with_castable_to_string_bound_accepts_infallible_cast() {
    let _ = parse_single_file_ast(
        r#"
render type T is CASTABLE_TO_STRING |value T| -> String:
    return cast value
;
"#,
    );
}

#[test]
fn generic_function_with_try_castable_to_int_bound_rejects_infallible_form() {
    assert_invalid_cast(
        r#"
parse type T is TRY_CASTABLE_TO_INT |value T| -> Int:
    return cast value
;
"#,
        InvalidCastReason::FallibleEvidenceRequiresHandling,
    );
}

#[test]
fn generic_function_with_try_castable_to_int_bound_accepts_cast_propagation() {
    let _ = parse_single_file_ast(
        r#"
parse type T is TRY_CASTABLE_TO_INT |value T| -> Int, Error!:
    return cast! value
;
"#,
    );
}

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
render type T is CASTABLE_TO_INT |value T| -> String:
    return cast value
;
"#,
        InvalidCastReason::NoEvidence,
    );
}

#[test]
fn concrete_generic_instance_emission_resolves_builtin_cast_evidence() {
    let _ = parse_single_file_ast(
        r#"
render type T is CASTABLE_TO_STRING |value T| -> String:
    return cast value
;

result = render(42)
"#,
    );
}

#[test]
fn concrete_generic_instance_emission_resolves_user_defined_cast_evidence() {
    let _ = parse_single_file_ast(
        r#"
UserId = |
    value String,
|

to_string |this UserId| -> String:
    return this.value
;

UserId must CASTABLE_TO_STRING

render type T is CASTABLE_TO_STRING |value T| -> String:
    return cast value
;

result = render(UserId(value = "abc"))
"#,
    );
}

// ------------------------
//  Cast fallible handling syntax
// ------------------------

#[test]
fn concrete_fallible_cast_accepts_attached_bang_propagation() {
    let _ = parse_single_file_ast(
        r#"
parse_count |text String| -> Int, Error!:
    return cast! text
;
"#,
    );
}

#[test]
fn concrete_fallible_cast_rejects_separated_bang() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> Int, Error!:
    return cast ! text
;
"#,
        InvalidCastReason::BangMustAttachToCast,
    );
}

#[test]
fn concrete_fallible_cast_accepts_recovery_suffix() {
    let _ = parse_single_file_ast(
        r#"
value Int = cast "42" catch:
    then 0
;
"#,
    );
}

#[test]
fn concrete_fallible_cast_rejects_propagation_recovery_conflict() {
    assert_invalid_cast(
        r#"
parse_count |text String| -> Int, Error!:
    return cast! text catch:
        then 0
    ;
;
"#,
        InvalidCastReason::PropagationAndRecoveryConflict,
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
