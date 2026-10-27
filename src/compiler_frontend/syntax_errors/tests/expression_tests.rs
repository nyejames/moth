use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast_diagnostic,
};

fn assert_common_syntax_mistake(source: &str, expected_reason_key: &'static str) {
    let failure = parse_single_file_ast_diagnostic(source);
    assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0031", Some(expected_reason_key));
}

#[test]
fn detects_double_equal_as_equality_mistake() {
    assert_common_syntax_mistake(
        "value = 1 == 2\n",
        "common_syntax_mistake.equality_operator",
    );
}

#[test]
fn detects_bang_equal_as_inequality_mistake() {
    assert_common_syntax_mistake(
        "value = 1 != 2\n",
        "common_syntax_mistake.inequality_operator",
    );
}

#[test]
fn detects_and_and_as_conjunction_mistake() {
    assert_common_syntax_mistake(
        "value = true && false\n",
        "common_syntax_mistake.logical_and_operator",
    );
}

#[test]
fn detects_or_or_as_disjunction_mistake() {
    assert_common_syntax_mistake(
        "value = true || false\n",
        "common_syntax_mistake.logical_or_operator",
    );
}

#[test]
fn detects_bang_as_boolean_negation_mistake() {
    assert_common_syntax_mistake(
        "value = !true\n",
        "common_syntax_mistake.boolean_bang_negation",
    );
}
