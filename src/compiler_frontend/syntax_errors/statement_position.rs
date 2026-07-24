//! Common language-mismatch mistakes in statement position.
//!
//! WHAT: Detects patterns like `// comment`, `fn name(...)`, `let x = ...`,
//! `match value {`, `else if`, and `struct Name { ... }` that users from other
//! languages write when they first encounter Moth.
//!
//! WHY: Statement position is where most structural syntax lives (declarations,
//! control flow, comments), so it is the richest source of language-mismatch errors.

use super::common_syntax_mistake;
use crate::compiler_frontend::compiler_messages::{CommonSyntaxMistakeReason, CompilerDiagnostic};
use crate::compiler_frontend::symbols::string_interning::StringId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Check for common statement-position mistakes before falling back to a generic error.
///
/// Called from the main body dispatch loop when a token does not match any known
/// statement start.
pub(crate) fn check_statement_common_mistake(
    token: &TokenKind,
    token_stream: &FileTokens,
) -> Option<CompilerDiagnostic> {
    let location = token_stream.current_location();

    match token {
        // `//` is integer division; comments use `--`
        TokenKind::IntDivide => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::StatementLineComment,
            location,
        )),

        // `!` in statement position (not fallible handling)
        TokenKind::Bang => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::BooleanBangNegation,
            location,
        )),

        _ => None,
    }
}

/// Check if a symbol is a common mistaken keyword from another language.
///
/// WHAT: `fn`, `function`, `def`, `let`, `var`, `const`, `match`, and `struct`
/// are not Moth keywords, but newcomers often type them out of habit.
///
/// Called from `parse_symbol_statement` before treating the symbol as a normal
/// variable reference, declaration, or call.
pub(crate) fn check_mistaken_keyword_symbol(
    symbol_id: StringId,
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> Option<CompilerDiagnostic> {
    let name = string_table.resolve(symbol_id);
    let location = token_stream.current_location();

    match name {
        "fn" | "function" | "def" => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::FunctionKeyword { keyword: symbol_id },
            location,
        )),

        "let" | "var" => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::LetOrVarKeyword,
            location,
        )),

        "const" => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::ConstKeyword,
            location,
        )),

        "match" => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::MatchKeyword,
            location,
        )),

        "struct" => Some(common_syntax_mistake(
            CommonSyntaxMistakeReason::StructKeyword { keyword: symbol_id },
            location,
        )),

        _ => None,
    }
}
