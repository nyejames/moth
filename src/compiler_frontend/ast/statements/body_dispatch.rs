//! Function-body statement dispatch loop.
//!
//! WHAT: routes one function/start-function token stream through statement-position parsing.
//! WHY: centralized dispatch keeps control flow readable while specialized helpers own detailed
//!      syntax handling (symbol statements, returns, expression statements).

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::{AstNode, NodeKind};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::statements::asserts::parse_assert_statement;
use crate::compiler_frontend::ast::statements::body_expr_stmt::parse_expression_statement_candidate;
use crate::compiler_frontend::ast::statements::body_return::parse_return_statement;
use crate::compiler_frontend::ast::statements::body_symbol::{
    parse_symbol_statement, parse_this_statement,
};
use crate::compiler_frontend::ast::statements::branching::create_branch;
use crate::compiler_frontend::ast::statements::diagnostics::{
    UnexpectedScopeCloseContext, unexpected_scope_close, unexpected_statement_token,
};
use crate::compiler_frontend::ast::statements::loops::create_loop;
use crate::compiler_frontend::ast::statements::match_arm_boundaries::{
    current_line_contains_top_level_fat_arrow, current_token_starts_match_arm_header,
};
use crate::compiler_frontend::ast::statements::scoped_blocks::{
    parse_scoped_block_statement, reserved_block_keyword_as_name_error,
};
use crate::compiler_frontend::ast::statements::value_production::{
    ProducedValues, ProducedValuesParseInput, ValueReceiverKind,
    is_missing_produced_value_boundary, parse_produced_values_typed,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DeferredFeatureReason, InvalidControlFlowStatementReason,
    InvalidFallibleHandlingReason, InvalidMatchArmReason, InvalidStandaloneStatementReason,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::syntax_errors::statement_position::check_statement_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::value_mode::ValueMode;
use crate::projects::settings;

/// AST body parsing carries authored diagnostics and internal retained-data failures separately.
///
/// The module emitter is the reporting boundary: it renders diagnostics and forwards
/// `CompilerError` values unchanged. Nested statement parsers therefore must not collapse this
/// result back into a diagnostic while constructing temporary frozen-table substreams.
type StatementDispatchResult<T> = Result<T, ExpressionParseError>;

fn statement_dispatch_error(diagnostic: CompilerDiagnostic) -> ExpressionParseError {
    diagnostic.into()
}

/// Produce a diagnostic for a deferred block keyword (`checked`, `async`).
///
/// If the keyword is followed by an assignment operator, treats it as an attempt to use the
/// keyword as a variable name and reports a reserved-name error instead.
fn deferred_block_error(
    token_stream: &FileTokens,
    string_table: &mut StringTable,
    keyword: &str,
    reason: DeferredFeatureReason,
) -> CompilerDiagnostic {
    let location = token_stream.current_location();
    if matches!(
        token_stream.peek_next_token(),
        Some(token) if token.is_assignment_operator()
    ) {
        let keyword_id = string_table.intern(keyword);
        return reserved_block_keyword_as_name_error(keyword_id, string_table, location);
    }

    CompilerDiagnostic::deferred_feature_reason(reason, location)
}

pub(crate) fn parse_function_body_statements(
    token_stream: &mut FileTokens,
    mut context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> StatementDispatchResult<Vec<AstNode>> {
    let mut body_nodes: Vec<AstNode> =
        Vec::with_capacity(token_stream.length / settings::TOKEN_TO_NODE_RATIO);

    while token_stream.index < token_stream.length {
        let current_token = token_stream.current_token_kind().to_owned();

        ast_log!("Parsing Token: ", #current_token);

        // Match-arm bodies end when the next line-initial arm header or `else` is reached.
        // Same-line accidental second arms are rejected here before statement dispatch.
        if context.kind == ContextKind::MatchArm {
            if let Some(candidate) = current_token_starts_match_arm_header(token_stream) {
                debug_assert_eq!(candidate.start_index, token_stream.index);
                debug_assert!(candidate.arrow_index > candidate.start_index);
                break;
            }

            if token_stream.current_token_kind() != &TokenKind::Else
                && current_line_contains_top_level_fat_arrow(token_stream)
            {
                return Err(statement_dispatch_error(
                    CompilerDiagnostic::invalid_match_arm(
                        InvalidMatchArmReason::ArmMustStartNewLine,
                        token_stream.current_location(),
                    ),
                ));
            }
        }

        match current_token {
            // Module start marker
            TokenKind::ModuleStart => {
                token_stream.advance();
            }

            // Symbol statements (declarations, assignments, calls)
            TokenKind::Symbol(_) => parse_symbol_statement(
                token_stream,
                &mut body_nodes,
                &mut context,
                type_interner,
                warnings,
                string_table,
            )?,

            TokenKind::This => parse_this_statement(
                token_stream,
                &mut body_nodes,
                &mut context,
                type_interner,
                string_table,
            )?,

            // Scoped blocks and deferred features
            TokenKind::Block => body_nodes.push(parse_scoped_block_statement(
                token_stream,
                &context,
                type_interner,
                warnings,
                string_table,
            )?),

            TokenKind::Checked => {
                return Err(statement_dispatch_error(deferred_block_error(
                    token_stream,
                    &mut *string_table,
                    "checked",
                    DeferredFeatureReason::CheckedBlock,
                )));
            }

            TokenKind::Async => {
                return Err(statement_dispatch_error(deferred_block_error(
                    token_stream,
                    &mut *string_table,
                    "async",
                    DeferredFeatureReason::AsyncBlock,
                )));
            }

            // Control flow
            TokenKind::Loop => {
                token_stream.advance();

                body_nodes.push(create_loop(
                    token_stream,
                    context.new_child_control_flow(ContextKind::Loop, string_table),
                    type_interner,
                    warnings,
                    string_table,
                )?);
            }

            TokenKind::If => {
                token_stream.advance();

                body_nodes.extend(create_branch(
                    token_stream,
                    &mut context.new_child_control_flow(ContextKind::Branch, string_table),
                    type_interner,
                    warnings,
                    string_table,
                )?);
            }

            TokenKind::Else => {
                if context.kind == ContextKind::Branch || context.kind == ContextKind::MatchArm {
                    break;
                } else {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::ElseOutsideIfOrMatch,
                            token_stream.current_location(),
                        ),
                    ));
                }
            }

            // Whitespace
            TokenKind::Newline => {
                token_stream.advance();
            }

            // Return, loop control, and result handling
            TokenKind::Assert => {
                parse_assert_statement(
                    token_stream,
                    &mut body_nodes,
                    &context,
                    type_interner,
                    string_table,
                )?;
            }

            TokenKind::Return | TokenKind::ReturnBang => {
                parse_return_statement(
                    token_stream,
                    &mut body_nodes,
                    &context,
                    type_interner,
                    string_table,
                )?;
            }

            TokenKind::Break => {
                if !context.is_inside_loop() {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::BreakOutsideLoop,
                            token_stream.current_location(),
                        ),
                    ));
                }

                body_nodes.push(AstNode {
                    kind: NodeKind::Break,
                    location: token_stream.current_location(),
                    scope: context.scope.clone(),
                });
                token_stream.advance();
            }

            TokenKind::Continue => {
                if !context.is_inside_loop() {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::ContinueOutsideLoop,
                            token_stream.current_location(),
                        ),
                    ));
                }

                body_nodes.push(AstNode {
                    kind: NodeKind::Continue,
                    location: token_stream.current_location(),
                    scope: context.scope.clone(),
                });
                token_stream.advance();
            }

            TokenKind::Then => {
                let then_location = token_stream.current_location();
                token_stream.advance();

                let Some(active_target) = &context.active_value_target else {
                    let reason = if matches!(
                        context.kind,
                        ContextKind::Loop
                            | ContextKind::Block
                            | ContextKind::CatchHandler
                            | ContextKind::Template
                    ) {
                        InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct
                    } else {
                        InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget
                    };

                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_fallible_handling(reason, then_location),
                    ));
                };

                if token_stream.current_token_kind() == &TokenKind::Newline
                    || is_missing_produced_value_boundary(token_stream.current_token_kind())
                {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::ThenRequiresValues,
                            token_stream.current_location(),
                        ),
                    ));
                }

                if active_target.result_type_ids.is_empty()
                    && active_target.receiver_kind != ValueReceiverKind::Declaration
                    && active_target.expected_arity.is_none()
                {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_fallible_handling(
                            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult,
                            then_location,
                        ),
                    ));
                }

                let produced_values = parse_produced_values_typed(ProducedValuesParseInput {
                    token_stream,
                    context: &context,
                    type_interner,
                    target: active_target,
                    label: "then fallback values",
                    string_table,
                })?;

                body_nodes.push(AstNode {
                    kind: NodeKind::ThenValue(ProducedValues {
                        expressions: produced_values,
                        location: then_location.clone(),
                    }),
                    location: then_location.clone(),
                    scope: context.scope.clone(),
                });
            }

            // Scope terminators
            TokenKind::End => match context.kind {
                ContextKind::Expression => {
                    return Err(statement_dispatch_error(unexpected_scope_close(
                        UnexpectedScopeCloseContext::Expression,
                        token_stream.current_location(),
                    )));
                }

                ContextKind::Template => {
                    return Err(statement_dispatch_error(unexpected_scope_close(
                        UnexpectedScopeCloseContext::Template,
                        token_stream.current_location(),
                    )));
                }

                ContextKind::MatchArm => break,

                _ => {
                    token_stream.advance();
                    break;
                }
            },

            // Templates
            // Top-level runtime template in the entry start() body.
            // Each template becomes a PushStartRuntimeFragment so the HIR builder can
            // push the evaluated string directly to the runtime fragment list.
            TokenKind::TemplateHead => {
                if context.kind != ContextKind::Module {
                    return Err(statement_dispatch_error(
                        CompilerDiagnostic::invalid_standalone_statement(
                            InvalidStandaloneStatementReason::StandaloneTemplate,
                            token_stream.current_location(),
                        ),
                    ));
                }

                let template = Template::new_with_type_interner(
                    token_stream,
                    &context,
                    type_interner,
                    vec![],
                    string_table,
                )?;
                let expression = Expression::template(template, ValueMode::MutableOwned);
                let location = token_stream.current_location();

                body_nodes.push(AstNode {
                    kind: NodeKind::PushStartRuntimeFragment(expression),
                    location,
                    scope: context.scope.clone(),
                })
            }

            // End of file
            TokenKind::Eof => {
                break;
            }

            // Expression statements
            TokenKind::OpenParenthesis
            | TokenKind::NumericLiteral(_)
            | TokenKind::StringSliceLiteral(_)
            | TokenKind::BoolLiteral(_)
            | TokenKind::CharLiteral(_)
            | TokenKind::Copy
            | TokenKind::Mutable => {
                let expression = parse_expression_statement_candidate(
                    token_stream,
                    &context,
                    type_interner,
                    string_table,
                )?;

                body_nodes.push(AstNode {
                    kind: NodeKind::ExpressionStatement(expression),
                    location: token_stream.current_location(),
                    scope: context.scope.clone(),
                });
            }

            // Unrecognized tokens
            _ => {
                if let Some(diagnostic) =
                    check_statement_common_mistake(token_stream.current_token_kind(), token_stream)
                {
                    return Err(statement_dispatch_error(diagnostic));
                }

                return Err(statement_dispatch_error(unexpected_statement_token(
                    token_stream.current_token_kind(),
                    token_stream.current_location(),
                    string_table,
                )));
            }
        }
    }

    warnings.extend(context.take_emitted_warnings());
    Ok(body_nodes)
}
