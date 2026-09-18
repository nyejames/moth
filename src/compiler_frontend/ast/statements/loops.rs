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
    ParsedLoopHeader, parse_loop_header_cursor,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidLoopHeaderReason};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

/// Stage-local result for loop statement AST construction.
///
/// WHY: loop-header parsing can detect an invalid frozen token-table lifecycle. That is an
/// internal compiler error, while ordinary loop syntax remains a typed source diagnostic.
type LoopResult<T> = Result<T, ExpressionParseError>;

/// Parse a complete `loop` statement after the `loop` keyword has been consumed.
pub fn create_loop(
    token_stream: &mut AstCursor,
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
    // Canonical header window only; compatibility streams have no owner to window.
    // Dense segmented coordinates stay window-bounded and canonical `SourceTokens`
    // are never cloned into a new `FileTokens` vector here.
    let Some(mut window) = token_stream.subcursor_window(start_index, colon_index)? else {
        return Err(CompilerError::compiler_error(
            "compatibility token stream cannot parse a loop header",
        )
        .into());
    };
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
    let (parsed_loop_header, body_context) = parse_loop_header_cursor(
        &mut window,
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

fn find_loop_header_colon_index(token_stream: &mut AstCursor) -> LoopResult<usize> {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut nesting_depth = NestingDepth::default();
    let mut outcome: Option<LoopResult<usize>> = None;
    while token_stream.position() < end && !token_stream.is_at_end() {
        // A malformed payload has no `TokenKind`; stop and report MissingColon at the
        // entry span, since no colon decision can be made past it.
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
        // A malformed payload has no `TokenKind`; stop and leave the emptiness decision to the header parser.
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

#[cfg(test)]
#[path = "tests/loop_parsing_tests.rs"]
mod loop_parsing_tests;
