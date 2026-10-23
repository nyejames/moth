//! Shared block-body parsing for value-producing `if` and match forms.
//!
//! WHAT: consumes `:`, installs the active value target on caller-supplied then
//! and else parents, parses both statement bodies, and requires `else`.
//! WHY: block Bool `if`, block single-predicate match, and inferred multi-bind
//! must not duplicate warning forwarding or `else` checking.

use super::emit_collected_warnings;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::numeric_construction::DeferredBody;
use crate::compiler_frontend::ast::statements::body_dispatch::{
    ParsedBody, parse_function_body_statements,
};
use crate::compiler_frontend::ast::statements::value_production::types::ActiveValueProductionTarget;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Input for the shared then/else block-body parser.
///
/// The then and else parents are already the correct branch scopes. This parser
/// creates one `Branch` child of each parent and installs the active target.
pub(in crate::compiler_frontend::ast::statements::value_production) struct BlockBodyParseInput<
    'a,
    'b,
    'tokens,
> {
    pub token_stream: &'a mut AstCursor<'tokens>,
    pub outer_context: &'a ScopeContext,
    pub then_parent: &'a ScopeContext,
    pub else_parent: &'a ScopeContext,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub string_table: &'a mut StringTable,
    pub active_target: ActiveValueProductionTarget,
    pub path_fork: &'a mut PathInternerFork,
}

pub(in crate::compiler_frontend::ast::statements::value_production) struct ParsedValueBlockBodies {
    pub then_body: ValueBlockBranchBody,
    pub else_body: ValueBlockBranchBody,
    pub generic_request_ranges: IfGenericRequestRanges,
}

impl ParsedValueBlockBodies {
    /// True when either branch body retained numeric continuations for the enclosing body root.
    pub(in crate::compiler_frontend::ast::statements::value_production) fn has_pending(
        &self,
    ) -> bool {
        self.then_body.has_pending() || self.else_body.has_pending()
    }
}

/// One value-block branch body before the enclosing body root resolves it.
///
/// WHAT: kept as authored statement nodes when the branch completed where it was parsed, or as
///       the numbered draft the branch parse returned when any statement still holds a numeric
///       continuation.
/// WHY: a concrete branch must not be re-wrapped or turned into a continuation, while a branch
///      that retained statements must keep them, with its scope, for body-root
///      resumption instead of being materialised early.
#[derive(Debug, Clone)]
pub(crate) enum ValueBlockBranchBody {
    /// The branch completed at its parse position; it publishes these nodes as authored.
    Nodes { nodes: Vec<AstNode>, scope: PathId },
    /// The branch retained numeric continuations for the enclosing body root.
    Draft(DeferredBody),
}

impl ValueBlockBranchBody {
    /// True when this branch retained at least one numeric continuation.
    pub(in crate::compiler_frontend::ast::statements::value_production) fn has_pending(
        &self,
    ) -> bool {
        matches!(self, Self::Draft(_))
    }

    /// Consume a completed branch as its authored statement nodes.
    ///
    /// WHAT: returns the branch nodes and a precise failure naming the position when the branch
    ///       still holds a retained statement.
    /// WHY: only a body that retained nothing may be published before the enclosing body root
    ///      completes; a retained statement must never be dropped or replaced by a placeholder.
    pub(in crate::compiler_frontend::ast::statements::value_production) fn into_nodes(
        self,
        position: &str,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        match self {
            Self::Nodes { nodes, .. } => Ok(nodes),
            Self::Draft(_) => Err(CompilerError::compiler_error(format!(
                "a numeric-dependent statement was retained inside {position} and cannot be \
                 published before the enclosing body root completes"
            ))
            .into()),
        }
    }

    /// The authored branch scope this body belongs to.
    pub(in crate::compiler_frontend::ast::statements::value_production) fn scope(&self) -> PathId {
        match self {
            Self::Nodes { scope, .. } => *scope,
            Self::Draft(body) => body.scope,
        }
    }
}

type BlockBodyResult<T> = Result<T, ExpressionParseError>;

/// Parses both value-block bodies after the header, starting at `:`.
pub(in crate::compiler_frontend::ast::statements::value_production) fn parse_value_block_bodies(
    input: BlockBodyParseInput<'_, '_, '_>,
) -> BlockBodyResult<ParsedValueBlockBodies> {
    let BlockBodyParseInput {
        token_stream,
        outer_context,
        then_parent,
        else_parent,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        active_target,
        path_fork,
    } = input;

    token_stream.advance(); // consume `:`

    let then_request_start = outer_context.generic_request_checkpoint();
    let then_body = parse_one_value_block_body(
        token_stream,
        then_parent,
        outer_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        active_target.clone(),
        path_fork,
    )?;
    let then_request_end = outer_context.generic_request_checkpoint();

    if token_stream.current_tag() != TokenTag::ELSE {
        return Err(choices_control_flow::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            Some(token_stream.current_span()),
        )
        .into());
    }
    token_stream.advance(); // consume `else`

    let else_body = parse_one_value_block_body(
        token_stream,
        else_parent,
        outer_context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        active_target,
        path_fork,
    )?;
    let else_request_end = outer_context.generic_request_checkpoint();

    Ok(ParsedValueBlockBodies {
        then_body,
        else_body,
        generic_request_ranges: IfGenericRequestRanges {
            then_branch: GenericRequestRange::new(then_request_start, then_request_end),
            else_branch: GenericRequestRange::new(then_request_end, else_request_end),
        },
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "branch body parsing keeps the cursor, both scopes, target and mutable state separate"
)]
fn parse_one_value_block_body(
    token_stream: &mut AstCursor,
    parent: &ScopeContext,
    outer_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    active_target: ActiveValueProductionTarget,
    path_fork: &mut PathInternerFork,
) -> BlockBodyResult<ValueBlockBranchBody> {
    let mut branch_context =
        parent.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
    let branch_scope = branch_context.scope;
    branch_context.active_value_target = Some(active_target);

    let mut warnings = DiagnosticBag::new();
    let parsed_body = parse_function_body_statements(
        token_stream,
        branch_context,
        type_interner,
        expression_builder,
        const_values,
        &mut warnings,
        string_table,
        path_fork,
    )?;
    emit_collected_warnings(outer_context, warnings);

    // A value-block branch shares the enclosing body root's numeric domain, so its parse returns
    // the authored-order draft. A draft that retained nothing publishes its nodes directly; one
    // that retained a statement keeps the whole body and its scope for body-root resumption
    // instead of being materialised early.
    Ok(match parsed_body {
        ParsedBody::Complete(nodes) => ValueBlockBranchBody::Nodes {
            nodes,
            scope: branch_scope,
        },
        ParsedBody::Draft(draft) => {
            if draft.has_pending() {
                ValueBlockBranchBody::Draft(DeferredBody {
                    draft,
                    scope: branch_scope,
                })
            } else {
                ValueBlockBranchBody::Nodes {
                    nodes: draft.into_nodes_for_host("a value-block branch body")?,
                    scope: branch_scope,
                }
            }
        }
    })
}
