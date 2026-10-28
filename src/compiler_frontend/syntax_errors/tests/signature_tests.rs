use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, parse_single_file_ast_diagnostic,
};

#[test]
fn detects_open_parenthesis_as_parameter_delimiter_mistake() {
    let failure =
        parse_single_file_ast_diagnostic("bad |a I32, (b I32)| -> I32:\n    return a\n;\n");

    assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0031",
        Some("common_syntax_mistake.signature_parenthesis_delimiter"),
    );
}
