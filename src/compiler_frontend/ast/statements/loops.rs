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
    ParsedLoopHeader, parse_loop_header_cursor, parse_loop_header_tokens,
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
    let colon_index = find_loop_header_colon_index(&mut *token_stream)?;

    let start_index = token_stream.position();
    // Canonical header window first; compatibility streams return `Ok(None)` and use the
    // explicit vector fallback below. Dense segmented coordinates stay window-bounded and
    // canonical `SourceTokens` are never cloned into a new `FileTokens` vector here.
    let (parsed_loop_header, body_context) =
        match token_stream.subcursor_window(start_index, colon_index)? {
            Some(mut window) => {
                if is_empty_header_window(&mut window) {
                    return Err(CompilerDiagnostic::invalid_loop_header(
                        InvalidLoopHeaderReason::EmptyHeader,
                        span,
                    )
                    .into());
                }
                let eof_span = window
                    .span_at(colon_index.saturating_sub(1))
                    .or(span)
                    .unwrap_or_else(|| token_stream.current_span());
                let mut window = window.with_synthetic_eof_span(eof_span);
                parse_loop_header_cursor(
                    &mut window,
                    context,
                    type_interner,
                    warnings,
                    string_table,
                    path_fork,
                )?
            }
            None => {
                let tokens =
                    collect_compatibility_header_tokens(token_stream, start_index, colon_index);
                if tokens
                    .iter()
                    .all(|token| matches!(token.kind, TokenKind::Newline))
                {
                    return Err(CompilerDiagnostic::invalid_loop_header(
                        InvalidLoopHeaderReason::EmptyHeader,
                        span,
                    )
                    .into());
                }
                parse_loop_header_tokens(
                    &tokens,
                    path_syntax,
                    context,
                    type_interner,
                    warnings,
                    string_table,
                    path_fork,
                )?
            }
        };

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

fn find_loop_header_colon_index(token_stream: &mut AstCursor) -> LoopResult<usize> {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut nesting_depth = NestingDepth::default();
    let mut outcome: Option<LoopResult<usize>> = None;
    while token_stream.position() < end && !token_stream.is_at_end() {
        // Mirror the indexed break: a malformed canonical payload reports MissingColon at
        // the entry span. Compatibility cursors have no TokenRef, so only a present
        // TokenRef can fail this check; compatibility reads its materialised kind below.
        if let Some(current) = token_stream.current()
            && current.to_token_kind().is_err()
        {
            break;
        }
        let kind = token_stream.current_token_kind().clone();
        let token_span = token_stream.current_span();
        let is_top_level = nesting_depth.is_top_level();

        if is_top_level && matches!(kind, TokenKind::Colon) {
            outcome = Some(Ok(token_stream.position()));
            break;
        }

        if is_top_level && matches!(kind, TokenKind::End | TokenKind::Eof) {
            outcome = Some(Err(CompilerDiagnostic::invalid_loop_header(
                InvalidLoopHeaderReason::MissingColon,
                Some(token_span),
            )
            .into()));
            break;
        }

        nesting_depth.step(&kind);
        let before = token_stream.position();
        token_stream.advance();
        // Eof never advances, so a stalled step ends the search.
        if token_stream.position() == before {
            break;
        }
    }

    token_stream
        .set_position(resume)
        .map_err(ExpressionParseError::from)?;
    outcome.unwrap_or_else(|| {
        Err(CompilerDiagnostic::invalid_loop_header(
            InvalidLoopHeaderReason::MissingColon,
            Some(token_stream.current_span()),
        )
        .into())
    })
}

/// Canonical empty-header check over the header window.
///
/// WHAT: reports whether the window holds only newlines (or nothing) without cloning tokens.
/// WHY: an empty header must report `EmptyHeader` instead of an expression diagnostic.
fn is_empty_header_window(window: &mut AstCursor) -> bool {
    let resume = window.position();
    let end = window.length();
    let mut empty = true;
    while window.position() < end && !window.is_at_end() {
        // A malformed payload has no `TokenKind`; the indexed scan stopped and reported empty.
        if window
            .current()
            .is_some_and(|current| current.to_token_kind().is_err())
        {
            break;
        }
        if *window.current_token_kind() != TokenKind::Newline {
            empty = false;
            break;
        }
        window.advance();
    }
    window
        .set_position(resume)
        .expect("empty header resume stays inside the active parser view");
    empty
}

/// Explicit compatibility fallback for the unowned legacy stream.
///
/// WHAT: collects `[start, end)` from the legacy token vector when `subcursor_window`
/// returns `Ok(None)`.
/// WHY: legacy `FileTokens` streams have no canonical owner to window; this lane
/// never clones canonical `SourceTokens`.
fn collect_compatibility_header_tokens(
    token_stream: &AstCursor,
    start: usize,
    end: usize,
) -> Vec<Token> {
    let mut header_tokens = Vec::new();
    let mut scan = start;
    while scan < end {
        let Some(token) = token_stream.token_at(scan) else {
            break;
        };
        header_tokens.push(token);
        scan += 1;
    }
    header_tokens
}

#[cfg(test)]
#[path = "tests/loop_parsing_tests.rs"]
mod loop_parsing_tests;
