use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, DiagnosticPayload};
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_diagnostic;

#[test]
fn detects_open_parenthesis_as_parameter_delimiter_mistake() {
    let diagnostic =
        parse_single_file_ast_diagnostic("bad |a I32, (b I32)| -> I32:\n    return a\n;\n");

    assert!(matches!(
        diagnostic.payload,
        DiagnosticPayload::CommonSyntaxMistake {
            reason: CommonSyntaxMistakeReason::SignatureParenthesisDelimiter
        }
    ));
}
