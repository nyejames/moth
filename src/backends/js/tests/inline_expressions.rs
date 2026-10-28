//! JavaScript inline external expression lowering tests.

use crate::backends::js::js_calls::substitute_inline_expression;

#[test]
fn inline_expression_substitutes_each_argument_once() {
    let args = vec![String::from("left"), String::from("right")];

    let result = substitute_inline_expression("Math.max(#0, #1)", &args)
        .expect("inline expression substitution should succeed");

    assert_eq!(result, "Math.max(left, right)");
}

#[test]
fn inline_expression_rejects_missing_argument_placeholder() {
    let args = vec![String::from("value"), String::from("fallback")];

    let error = substitute_inline_expression("Math.abs(#0)", &args)
        .expect_err("missing argument placeholder should fail");

    assert!(
        error.message.contains("missing placeholder '#1'"),
        "expected missing placeholder diagnostic, got: {}",
        error.message
    );
}

#[test]
fn inline_expression_rejects_duplicate_argument_placeholder() {
    let args = vec![String::from("value")];

    let error = substitute_inline_expression("(#0 + #0)", &args)
        .expect_err("duplicate argument placeholder should fail");

    assert!(
        error.message.contains("duplicate placeholder '#0'"),
        "expected duplicate placeholder diagnostic, got: {}",
        error.message
    );
}

#[test]
fn inline_expression_rejects_stray_placeholder() {
    let args = vec![String::from("left"), String::from("right")];

    let error = substitute_inline_expression("Math.min(#0, #1, #2)", &args)
        .expect_err("placeholder without a matching argument should fail");

    assert!(
        error.message.contains("placeholder '#2'"),
        "expected stray placeholder diagnostic, got: {}",
        error.message
    );
}
