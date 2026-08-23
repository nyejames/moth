//! Shared block-body parsing for value-producing `if` and match forms.
//!
//! WHAT: consumes `:`, installs the active value target on caller-supplied then
//! and else parents, parses both statement bodies, and requires `else`.
//! WHY: block Bool `if`, later block match, and inferred multi-bind must not
//! duplicate warning forwarding or `else` checking.

use super::emit_collected_warnings;
use crate::compiler_frontend::ast::ContextKind;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::statements::body_dispatch::parse_function_body_statements;
use crate::compiler_frontend::ast::statements::value_production::completeness::analyze_branch_exits;
use crate::compiler_frontend::ast::statements::value_production::types::{
    ActiveValueProductionTarget, BranchExitSummary,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};

/// Input for the shared then/else block-body parser.
///
/// The then and else parents are already the correct branch scopes. This parser
/// creates one `Branch` child of each parent and installs the active target.
pub(in crate::compiler_frontend::ast::statements::value_production) struct BlockBodyParseInput<
    'a,
    'b,
> {
    pub token_stream: &'a mut FileTokens,
    pub outer_context: &'a ScopeContext,
    pub then_parent: &'a ScopeContext,
    pub else_parent: &'a ScopeContext,
    pub type_interner: &'a mut AstTypeInterner<'b>,
    pub string_table: &'a mut StringTable,
    pub active_target: ActiveValueProductionTarget,
}

/// Parsed then and else bodies plus their all-path exit summaries.
pub(in crate::compiler_frontend::ast::statements::value_production) struct ParsedValueBlockBodies {
    pub then_body: Vec<AstNode>,
    pub else_body: Vec<AstNode>,
    pub then_exits: BranchExitSummary,
    pub else_exits: BranchExitSummary,
}

type BlockBodyResult<T> = Result<T, ExpressionParseError>;

/// Parses both value-block bodies after the header, starting at `:`.
pub(in crate::compiler_frontend::ast::statements::value_production) fn parse_value_block_bodies(
    input: BlockBodyParseInput<'_, '_>,
) -> BlockBodyResult<ParsedValueBlockBodies> {
    let BlockBodyParseInput {
        token_stream,
        outer_context,
        then_parent,
        else_parent,
        type_interner,
        string_table,
        active_target,
    } = input;

    token_stream.advance(); // consume `:`

    let then_body = parse_one_value_block_body(
        token_stream,
        then_parent,
        outer_context,
        type_interner,
        string_table,
        active_target.clone(),
    )?;

    if token_stream.current_token_kind() != &TokenKind::Else {
        return Err(CompilerDiagnostic::invalid_control_flow_statement(
            InvalidControlFlowStatementReason::ValueIfMissingElse,
            token_stream.current_location(),
        )
        .into());
    }
    token_stream.advance(); // consume `else`

    let else_body = parse_one_value_block_body(
        token_stream,
        else_parent,
        outer_context,
        type_interner,
        string_table,
        active_target,
    )?;

    let then_exits = analyze_branch_exits(&then_body);
    let else_exits = analyze_branch_exits(&else_body);

    Ok(ParsedValueBlockBodies {
        then_body,
        else_body,
        then_exits,
        else_exits,
    })
}

fn parse_one_value_block_body(
    token_stream: &mut FileTokens,
    parent: &ScopeContext,
    outer_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    string_table: &mut StringTable,
    active_target: ActiveValueProductionTarget,
) -> BlockBodyResult<Vec<AstNode>> {
    let mut branch_context = parent.new_child_control_flow(ContextKind::Branch, string_table);
    branch_context.active_value_target = Some(active_target);

    let mut warnings = Vec::new();
    let body = parse_function_body_statements(
        token_stream,
        branch_context,
        type_interner,
        &mut warnings,
        string_table,
    )?;
    emit_collected_warnings(outer_context, warnings);

    Ok(body)
}
