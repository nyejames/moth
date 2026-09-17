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
    // Canonical header window first; compatibility streams return `Ok(None)` and use the
    // explicit vector fallback below. Dense segmented coordinates stay window-bounded and
    // canonical `SourceTokens` are never cloned into a new `FileTokens` vector here.
    let header_window = token_stream.subcursor_window(start_index, colon_index)?;
    let header_tokens: Vec<Token> = match &header_window {
        Some(window) => {
            if is_empty_header_window(window, start_index, colon_index) {
                return Err(CompilerDiagnostic::invalid_loop_header(
                    InvalidLoopHeaderReason::EmptyHeader,
                    span,
                )
                .into());
            }
            collect_header_window_for_grammar(window, start_index, colon_index)
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
            tokens
        }
    };

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

/// Bounded canonical empty-header check over `[start, end)`.
///
/// WHAT: reports whether the window holds only newlines (or nothing) without cloning tokens.
/// WHY: the canonical header view must not read outside its half-open window; dense segmented
/// positions stay window-relative through `token_kind_at`.
fn is_empty_header_window(window: &AstCursor, start: usize, end: usize) -> bool {
    // Match the legacy scan: a truncated window reports empty only when every readable token
    // was a newline. The half-open bound is unchanged; the window already rejects reads at
    // or beyond `end`.
    let mut scan = start;
    while scan < end {
        match window.token_kind_at(scan) {
            Some(TokenKind::Newline) => scan += 1,
            Some(_) => return false,
            None => break,
        }
    }
    true
}

/// Explicit grammar adapter: materialize the canonical window for `parse_loop_header_tokens`.
///
/// WHAT: copies only `[start, end)` into the transient `&[Token]` grammar boundary.
/// WHY: loop-header internals still own the `&[Token]` grammar handoff (next task converts
/// them); no `SourceTokens` are cloned into a new `FileTokens` vector here and no cursor is
/// retained.
fn collect_header_window_for_grammar(window: &AstCursor, start: usize, end: usize) -> Vec<Token> {
    let mut header_tokens = Vec::new();
    let mut scan = start;
    while scan < end {
        let Some(token) = window.token_at(scan) else {
            break;
        };
        header_tokens.push(token);
        scan += 1;
    }
    header_tokens
}

/// Explicit compatibility fallback for unowned/synthetic streams.
///
/// WHAT: collects `[start, end)` from the legacy token vector when `subcursor_window`
/// returns `Ok(None)`.
/// WHY: synthetic/remapped `FileTokens` streams have no canonical owner to window; this lane
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
