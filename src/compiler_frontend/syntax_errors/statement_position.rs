//! Common language-mismatch mistakes in statement position.
//!
//! WHAT: Detects patterns like `// comment`, `fn name(...)`, `let x = ...`,
//! `match value {`, `else if`, and `struct Name { ... }` that users from other
//! languages write when they first encounter Moth.
//!
//! WHY: Statement position is where most structural syntax lives (declarations,
//! control flow, comments), so it is the richest source of language-mismatch errors.

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::compiler_messages::CommonSyntaxMistakeReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Check for common statement-position mistakes before falling back to a generic error.
///
/// Called from the main body dispatch loop when a token does not match any known
/// statement start.
pub(crate) fn check_statement_common_mistake(
    token_tag: TokenTag,
    token_stream: &AstCursor,
) -> Option<DiagnosticDraft> {
    let location = token_stream.current_span();

    match token_tag {
        // `//` is integer division; comments use `--`
        TokenTag::INT_DIVIDE => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::StatementLineComment,
            Some(location),
        )),

        // `!` in statement position (not fallible handling)
        TokenTag::BANG => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::BooleanBangNegation,
            Some(location),
        )),

        _ => None,
    }
}

/// Check if a symbol is a common mistaken keyword from another language.
///
/// WHAT: `function`, `def`, `let`, `var`, `const`, `match`, and `struct`
/// are not Moth keywords, but newcomers often type them out of habit.
///
/// Called from `parse_symbol_statement` before treating the symbol as a normal
/// variable reference, declaration, or call.
pub(crate) fn check_mistaken_keyword_symbol(
    symbol_id: StringId,
    token_stream: &AstCursor,
    string_table: &StringTable,
) -> Option<DiagnosticDraft> {
    let name = string_table.resolve(symbol_id);
    let location = token_stream.current_span();

    match name {
        "function" | "def" => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::FunctionKeyword { keyword: symbol_id },
            Some(location),
        )),

        "let" | "var" => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::LetOrVarKeyword,
            Some(location),
        )),

        "const" => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::ConstKeyword,
            Some(location),
        )),

        "match" => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::MatchKeyword,
            Some(location),
        )),

        "struct" => Some(token_lexical::common_syntax_mistake(
            CommonSyntaxMistakeReason::StructKeyword { keyword: symbol_id },
            Some(location),
        )),

        _ => None,
    }
}
