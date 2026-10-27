use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast_diagnostic,
};

fn assert_common_syntax_mistake(source: &str, expected_reason_key: &'static str) {
    let failure = parse_single_file_ast_diagnostic(source);

    assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0031", Some(expected_reason_key));
}

#[test]
fn detects_int_divide_at_statement_start_as_comment_mistake() {
    assert_common_syntax_mistake(
        "// comment\n",
        "common_syntax_mistake.statement_line_comment",
    );
}

#[test]
fn detects_fn_keyword_as_function_declaration_mistake() {
    assert_common_syntax_mistake("fn hello():\n", "common_syntax_mistake.function_keyword");
}

#[test]
fn detects_let_keyword_as_declaration_mistake() {
    assert_common_syntax_mistake("let x = 1\n", "common_syntax_mistake.let_or_var_keyword");
}

#[test]
fn detects_match_keyword_as_pattern_matching_mistake() {
    assert_common_syntax_mistake("match x:\n", "common_syntax_mistake.match_keyword");
}

#[test]
fn detects_struct_keyword_as_struct_declaration_mistake() {
    assert_common_syntax_mistake("struct Name { }\n", "common_syntax_mistake.struct_keyword");
}
