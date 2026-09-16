//! Loop statement AST construction.
//!
//! WHAT: finds a statement loop header, delegates body-independent header parsing
//! to `loop_headers`, then parses the statement body into the correct AST loop node.
//! WHY: template loop suffixes need to reuse loop-header syntax without inheriting
//! statement-body parsing.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::statements::loop_headers::{
    ParsedLoopHeader, parse_loop_header_tokens,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidLoopHeaderReason};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FilePathSyntax, Token, TokenKind};
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

/// Stage-local result for loop statement AST construction.
///
/// WHY: loop-header parsing can detect an invalid frozen token-table lifecycle. That is an
/// internal compiler error, while ordinary loop syntax remains a typed source diagnostic.
type LoopResult<T> = Result<T, ExpressionParseError>;

/// Parse a complete `loop` statement after the `loop` keyword has been consumed.
pub fn create_loop(
    token_stream: &mut AstCursor,
    path_syntax: &FilePathSyntax,
    context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> LoopResult<AstNode> {
    ast_log!("Creating a Loop");

    let span = token_stream.previous_span();
    let scope = context.scope;
    let colon_index = find_loop_header_colon_index(token_stream)?;

    let start_index = token_stream.position();
    let mut header_tokens: Vec<Token> = Vec::new();
    let mut scan = start_index;
    while scan < colon_index {
        let Some(token) = token_stream.token_at(scan) else {
            break;
        };
        header_tokens.push(token);
        scan += 1;
    }
    if header_tokens
        .iter()
        .all(|token| matches!(token.kind, TokenKind::Newline))
    {
        return Err(CompilerDiagnostic::invalid_loop_header(
            InvalidLoopHeaderReason::EmptyHeader,
            span,
        )
        .into());
    }

    let (parsed_loop_header, body_context) = parse_loop_header_tokens(
        &header_tokens,
        path_syntax,
        context,
        type_interner,
        warnings,
        string_table,
        path_fork,
    )?;

    token_stream.set_position(colon_index.saturating_add(1))?;

    let body = function_body_to_ast(
        token_stream,
        body_context,
        type_interner,
        warnings,
        string_table,
        path_fork,
    )?;

    let kind = match parsed_loop_header {
        ParsedLoopHeader::Conditional { condition } => NodeKind::WhileLoop(condition, body),
        ParsedLoopHeader::Range { bindings, range } => NodeKind::RangeLoop {
            bindings,
            range,
            body,
        },
        ParsedLoopHeader::Collection { bindings, iterable } => NodeKind::CollectionLoop {
            bindings,
            iterable,
            body,
        },
    };

    Ok(AstNode { kind, span, scope })
}

fn find_loop_header_colon_index(token_stream: &AstCursor) -> LoopResult<usize> {
    let mut nesting_depth = NestingDepth::default();
    let mut search_index = token_stream.position();

    while search_index < token_stream.length() {
        let Some(kind) = token_stream.token_kind_at(search_index) else {
            break;
        };
        let Some(token_span) = token_stream.span_at(search_index) else {
            break;
        };
        let is_top_level = nesting_depth.is_top_level();

        if is_top_level && matches!(kind, TokenKind::Colon) {
            return Ok(search_index);
        }

        if is_top_level && matches!(kind, TokenKind::End | TokenKind::Eof) {
            return Err(CompilerDiagnostic::invalid_loop_header(
                InvalidLoopHeaderReason::MissingColon,
                Some(token_span),
            )
            .into());
        }

        nesting_depth.step(&kind);
        search_index += 1;
    }

    Err(CompilerDiagnostic::invalid_loop_header(
        InvalidLoopHeaderReason::MissingColon,
        Some(token_stream.current_span()),
    )
    .into())
}

#[cfg(test)]
#[path = "tests/loop_parsing_tests.rs"]
mod loop_parsing_tests;
