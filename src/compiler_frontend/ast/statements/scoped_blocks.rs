//! Keyword-led scoped block parsing.
//!
//! WHAT: parses ordinary `block:` statements into AST nodes with their own lexical parser scope.
//! WHY: scoped blocks are statement syntax, not symbol-led declarations or labels.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext, function_body_to_ast};
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, ReservedNameOwner};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};

/// Scoped blocks recurse into the AST body parser, so they preserve its diagnostic and
/// infrastructure lanes until module emission.
type ScopedBlockResult<T> = Result<T, ExpressionParseError>;

/// Build a diagnostic for using a reserved block keyword (e.g. `block`) as a variable name.
///
/// `string_table` is accepted to keep the signature consistent with callers in
/// `body_dispatch.rs` that intern the keyword locally.
pub(crate) fn reserved_block_keyword_as_name_error(
    keyword: StringId,
    _string_table: &StringTable,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::reserved_name_collision(keyword, ReservedNameOwner::Keyword, location)
}

pub(crate) fn parse_scoped_block_statement(
    token_stream: &mut FileTokens,
    parent_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> ScopedBlockResult<AstNode> {
    let statement_location = token_stream.current_location();
    token_stream.advance();

    match token_stream.current_token_kind() {
        TokenKind::Colon => token_stream.advance(),

        operator_token if operator_token.is_assignment_operator() => {
            let block_keyword_id = string_table.intern("block");
            return Err(reserved_block_keyword_as_name_error(
                block_keyword_id,
                string_table,
                statement_location,
            )
            .into());
        }

        unexpected_token_kind => {
            return Err(CompilerDiagnostic::expected_token(
                TokenKind::Colon,
                Some(unexpected_token_kind.clone()),
                token_stream.current_location(),
            )
            .into());
        }
    }

    // Create a child control-flow context so the block body gets its own lexical scope.
    let block_context = parent_context.new_child_control_flow(ContextKind::Block, string_table);
    let block_scope = block_context.scope.clone();

    // Parse the block body against the child context.
    let body = function_body_to_ast(
        token_stream,
        block_context,
        type_interner,
        warnings,
        string_table,
    )?;

    // Emit the scoped block node, preserving the body's scope for later name resolution.
    Ok(AstNode {
        kind: NodeKind::ScopedBlock { body },
        location: statement_location,
        scope: block_scope,
    })
}

#[cfg(test)]
#[path = "tests/scoped_blocks_tests.rs"]
mod scoped_blocks_tests;
