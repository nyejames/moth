//! Loop statement AST construction.
//!
//! WHAT: finds a statement loop header, delegates body-independent header parsing
//! to `loop_headers`, then parses the statement body into the correct AST loop node.
//! WHY: template loop suffixes need to reuse loop-header syntax without inheriting
//! statement-body parsing.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::const_values::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, DeferredBody, DeferredConstruct, DeferredLoop, DeferredLoopHeader,
    DeferredRangeHeader, NumericContinuation, StatementExpression,
};
use crate::compiler_frontend::ast::statements::body_dispatch::{
    ParsedBody, parse_function_body_statements,
};
use crate::compiler_frontend::ast::statements::loop_headers::{
    ParsedLoopHeader, ParsedRangeHeader, parse_loop_header_cursor,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, InvalidLoopHeaderReason,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::NestingDepth;

/// Stage-local result for loop statement AST construction.
///
/// WHY: loop-header parsing can detect an invalid frozen token-table lifecycle. That is an
/// internal compiler error, while ordinary loop syntax remains a typed source diagnostic.
type LoopResult<T> = Result<T, ExpressionParseError>;

/// One parsed loop body: finished nodes, or a retained draft awaiting numeric completion.
///
/// WHAT: a nested loop body shares the enclosing function's construction domain, so it returns
///       its authored draft until the body root completes every retained origin.
/// WHY: the loop statement must retain the exact parsed body instead of re-parsing tokens or
///      defaulting the origins it references.
enum LoopBody {
    Complete(Vec<AstNode>),
    Retained(DeferredBody),
}

impl LoopBody {
    /// Bridge one nested body parse into the loop's own body representation.
    fn from_parsed(parsed: ParsedBody, scope: PathId) -> LoopResult<Self> {
        match parsed {
            ParsedBody::Complete(nodes) => Ok(LoopBody::Complete(nodes)),
            ParsedBody::Draft(draft) => {
                if draft.has_pending() {
                    Ok(LoopBody::Retained(DeferredBody { draft, scope }))
                } else {
                    Ok(LoopBody::Complete(
                        draft.into_nodes_for_host("a loop body")?,
                    ))
                }
            }
        }
    }

    fn is_retained(&self) -> bool {
        matches!(self, LoopBody::Retained(_))
    }

    /// Move one parsed body into the loop's retained representation.
    ///
    /// WHAT: a retained body is handed over as-is; a body whose statements all completed in place
    ///       becomes an authored draft of those nodes.
    /// WHY: a loop whose header still needs numeric completion must reach the body root as one
    ///      retained loop even when its body needed no completion, so the header's operands can
    ///      select their types there instead of failing as an unsupported source position.
    fn into_deferred(self, scope: PathId) -> DeferredBody {
        match self {
            LoopBody::Complete(nodes) => {
                let mut draft = BodyDraft::new();
                draft.extend_nodes(nodes);
                DeferredBody { draft, scope }
            }
            LoopBody::Retained(body) => body,
        }
    }

    fn into_nodes(self) -> LoopResult<Vec<AstNode>> {
        match self {
            LoopBody::Complete(nodes) => Ok(nodes),
            LoopBody::Retained(_) => Err(CompilerError::compiler_error(
                "a loop body was retained after the loop decided it needs no numeric completion",
            )
            .into()),
        }
    }
}

/// Parse a complete `loop` statement after the `loop` keyword has been consumed.
///
/// WHAT: pushes the finished loop node, or records one retained construct when the header
///       condition, the header operands or the body still hold numeric-dependent positions. A
///       loop whose header needs completion is retained even when its body parsed every statement
///       in place, and the body then reaches the body root as an authored draft.
/// WHY: a nested body's pending positions resolve at the body root, so the loop must keep its
///      authored header, body draft and request boundaries until then.
#[allow(
    clippy::too_many_arguments,
    reason = "loop construction keeps the token stream, scope, the shared type, expression and constant stores, warnings, string and path services, and the body draft as separate borrows"
)]
pub fn create_loop(
    token_stream: &mut AstCursor,
    context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    draft: &mut BodyDraft,
) -> LoopResult<()> {
    ast_log!("Creating a Loop");

    let span = token_stream.previous_span();
    let scope = context.scope;
    // The loop's retained construct registers in the enclosing body's construction domain, while
    // the header and body parses consume this context; keeping the shared handle registers the
    // continuation without cloning the scope payload or re-deriving the domain.
    let numeric_construction = context.numeric_construction_handle();
    let start_index = token_stream.position();
    let colon_index = find_loop_header_colon_index(&mut *token_stream)?;

    let mut window = token_stream.subcursor_window(start_index, colon_index)?;
    if is_empty_header_window(&mut window)? {
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
    let mut header_warnings = Vec::new();
    let header_result = parse_loop_header_cursor(
        &mut window,
        context,
        type_interner,
        expression_builder,
        const_values,
        &mut header_warnings,
        string_table,
        path_fork,
    );
    warnings.append(DiagnosticBag::from_diagnostics(header_warnings));
    let (parsed_loop_header, body_context) = header_result?;

    token_stream.set_position(colon_index.saturating_add(1))?;

    let body_scope = body_context.scope;
    let parsed_body = parse_function_body_statements(
        token_stream,
        body_context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        string_table,
        path_fork,
    )?;
    let body = LoopBody::from_parsed(parsed_body, body_scope)?;

    if header_needs_completion(&parsed_loop_header) || body.is_retained() {
        let body = body.into_deferred(body_scope);
        let deferred = DeferredLoop {
            header: deferred_loop_header(parsed_loop_header),
            body,
            span,
            scope,
        };
        let continuation = numeric_construction
            .borrow_mut()
            .register_continuation(NumericContinuation::Construct(Box::new(
                DeferredConstruct::Loop(Box::new(deferred)),
            )))
            .map_err(ExpressionParseError::from)?;
        draft.push_pending(continuation);
        return Ok(());
    }

    let body = body.into_nodes()?;
    let kind = match parsed_loop_header {
        ParsedLoopHeader::Conditional { condition } => {
            let StatementExpression::Built(condition) = condition else {
                return Err(CompilerError::compiler_error(
                    "a loop condition was retained after the loop decided it needs no numeric completion",
                )
                .into());
            };
            NodeKind::WhileLoop(condition, body)
        }
        ParsedLoopHeader::Range(header) => {
            let ParsedRangeHeader::Concrete { bindings, range } = header else {
                return Err(CompilerError::compiler_error(
                    "a range header was retained after the loop decided it needs no numeric completion",
                )
                .into());
            };
            NodeKind::RangeLoop {
                bindings,
                range,
                body,
            }
        }
        ParsedLoopHeader::Collection {
            bindings,
            iterable,
            pending_item,
        } => {
            let (StatementExpression::Built(iterable), None) = (iterable, pending_item) else {
                return Err(CompilerError::compiler_error(
                    "a collection loop source was retained after the loop decided it needs no numeric completion",
                )
                .into());
            };
            NodeKind::CollectionLoop {
                bindings,
                iterable,
                body,
            }
        }
    };

    draft.push_node(AstNode { kind, span, scope });
    Ok(())
}

/// True when one parsed loop header still holds a numeric-dependent position.
fn header_needs_completion(header: &ParsedLoopHeader) -> bool {
    match header {
        ParsedLoopHeader::Conditional { condition } => {
            matches!(condition, StatementExpression::Retained(_))
        }
        ParsedLoopHeader::Range(header) => matches!(header, ParsedRangeHeader::Deferred(_)),
        ParsedLoopHeader::Collection { iterable, .. } => {
            matches!(iterable, StatementExpression::Retained(_))
        }
    }
}

/// Move one parsed loop header into its retained representation.
fn deferred_loop_header(header: ParsedLoopHeader) -> DeferredLoopHeader {
    match header {
        ParsedLoopHeader::Conditional { condition } => {
            DeferredLoopHeader::Conditional { condition }
        }
        ParsedLoopHeader::Range(header) => match header {
            // A concrete header whose loop stayed retained because of its body: the spec and its
            // bindings are final, so the body root only assembles the node.
            ParsedRangeHeader::Concrete { bindings, range } => {
                DeferredLoopHeader::Range(Box::new(DeferredRangeHeader::Ready { bindings, range }))
            }
            ParsedRangeHeader::Deferred(header) => DeferredLoopHeader::Range(Box::new(header)),
        },
        ParsedLoopHeader::Collection {
            bindings,
            iterable,
            pending_item,
        } => DeferredLoopHeader::Collection {
            bindings,
            iterable,
            pending_item,
        },
    }
}

fn find_loop_header_colon_index(token_stream: &mut AstCursor) -> LoopResult<usize> {
    let resume = token_stream.position();
    let end = token_stream.length();
    let mut nesting_depth = NestingDepth::default();
    let mut outcome: Option<LoopResult<usize>> = None;
    while token_stream.position() < end && !token_stream.is_at_end() {
        token_stream.validate_current_payload()?;
        let tag = token_stream.current_tag();
        let token_span = token_stream.current_span();
        let is_top_level = nesting_depth.is_top_level();

        if is_top_level && tag == TokenTag::COLON {
            outcome = Some(Ok(token_stream.position()));
            break;
        }

        if is_top_level && matches!(tag, TokenTag::END | TokenTag::EOF) {
            outcome = Some(Err(CompilerDiagnostic::invalid_loop_header(
                InvalidLoopHeaderReason::MissingColon,
                Some(token_span),
            )
            .into()));
            break;
        }

        nesting_depth.step_tag(tag);
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
fn is_empty_header_window(window: &mut AstCursor) -> LoopResult<bool> {
    let resume = window.position();
    let end = window.length();
    let mut empty = true;
    while window.position() < end && !window.is_at_end() {
        window.validate_current_payload()?;
        if window.current_tag() != TokenTag::NEWLINE {
            empty = false;
            break;
        }
        window.advance();
    }
    window
        .set_position(resume)
        .expect("empty header resume stays inside the active parser view");
    Ok(empty)
}

#[cfg(test)]
#[path = "tests/loop_parsing_tests.rs"]
mod loop_parsing_tests;
