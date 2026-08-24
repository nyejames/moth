//! Match and if/else branching AST construction.
//!
//! WHAT: parses `if`/`else` conditionals and `if value is:` match statements
//! into AST branch and match nodes.
//! WHY: this module owns statement body parsing and final `NodeKind::Match`
//! construction, while shared match helpers provide reusable header parsing and
//! exhaustiveness validation for later template control-flow work.

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, IfBranchMetadata, MatchExhaustiveness, NodeKind,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::Expression;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::statements::if_headers::{ParsedIfHeader, parse_if_header};
use crate::compiler_frontend::ast::statements::match_arm_boundaries::{
    current_line_contains_top_level_colon, current_token_starts_match_arm_header,
    token_index_has_top_level_fat_arrow, token_is_line_initial,
};
use crate::compiler_frontend::ast::statements::match_exhaustiveness::{
    MatchArmCoverageTracker, MatchExhaustivenessCheck, enforce_match_exhaustiveness,
};
use crate::compiler_frontend::ast::statements::match_headers::{
    ParsedMatchArmHeader, parse_match_arm_header,
};
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::ActiveValueProductionTarget;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidControlFlowStatementReason, InvalidMatchArmReason,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token, TokenKind};

/// Branches recursively parse function bodies, so retained-data failures travel to the module
/// emission boundary instead of being recast as authored control-flow diagnostics.
type BranchingResult<T> = Result<T, ExpressionParseError>;

fn branching_error(diagnostic: CompilerDiagnostic) -> ExpressionParseError {
    diagnostic.into()
}

/// Intermediate result of parsing a single match arm, carrying the arm itself
/// plus metadata needed for duplicate and exhaustiveness checking.
struct ParsedMatchArm {
    arm: MatchArm,
    /// Tracks which choice variant this arm consumes so duplicates can be rejected early.
    matched_choice_variant: Option<StringId>,
    pattern_location: SourceLocation,
}

struct OptionPresentCaptureBranch {
    scrutinee: Expression,
    pattern: MatchPattern,
    then_context: ScopeContext,
}

/// Parsed full-match block payload shared by statement and value-producing matches.
///
/// WHAT: carries the arm/default bodies plus the exhaustiveness contract after all
/// pattern parsing and validation has completed.
/// WHY: value-producing matches must reuse statement match parsing rules without
/// manufacturing a temporary statement node.
pub(crate) struct ParsedMatchBlock {
    pub scrutinee: Expression,
    pub arms: Vec<MatchArm>,
    pub default: Option<Vec<AstNode>>,
    pub exhaustiveness: MatchExhaustiveness,
    pub location: SourceLocation,
    pub scope: InternedPath,
}

/// Peek at the next non-newline token without advancing the stream.
fn peek_next_non_newline_token(token_stream: &FileTokens) -> Option<&Token> {
    token_stream
        .tokens
        .iter()
        .skip(token_stream.index + 1)
        .find(|token| token.kind != TokenKind::Newline)
}

/// Peek at the index of the next non-newline token without advancing the stream.
fn peek_next_non_newline_token_index(token_stream: &FileTokens) -> Option<usize> {
    token_stream
        .tokens
        .iter()
        .enumerate()
        .skip(token_stream.index + 1)
        .find(|(_, token)| token.kind != TokenKind::Newline)
        .map(|(i, _)| i)
}

fn reject_same_line_else_if(token_stream: &FileTokens) -> BranchingResult<()> {
    let else_location = token_stream.current_location();
    let Some(next_token) = token_stream.tokens.get(token_stream.index + 1) else {
        return Ok(());
    };

    // Statement `else if` is deliberately not a branch-chain syntax. A nested
    // `if` remains available as the first statement inside a separate `else` body.
    let next_token_is_same_line_if = matches!(next_token.kind, TokenKind::If)
        && next_token.location.start_pos.line_number == else_location.start_pos.line_number;

    if next_token_is_same_line_if {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ElseIfUnsupported,
                else_location,
            ),
        ));
    }

    Ok(())
}

/// Parse an `if` statement or an `if <subject> is:` match statement.
///
/// WHAT: detects single-predicate option matches (`if option is |name|:`) before
/// falling back to normal `if`/`else` parsing or statement-style match parsing.
/// WHY: the single-predicate and statement-match shapes share the `if` keyword,
/// so this entry point disambiguates them early based on token lookahead.
pub fn create_branch(
    token_stream: &mut FileTokens,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> BranchingResult<Vec<AstNode>> {
    let parsed_header = parse_if_header(token_stream, context, type_interner, string_table)?;

    let condition = match parsed_header {
        ParsedIfHeader::OptionPresentCapture {
            scrutinee,
            pattern,
            then_context,
        } => {
            return create_option_present_capture_branch(
                OptionPresentCaptureBranch {
                    scrutinee,
                    pattern,
                    then_context,
                },
                token_stream,
                context,
                type_interner,
                warnings,
                string_table,
            );
        }
        ParsedIfHeader::MatchStyle { scrutinee } => {
            let match_statement = create_match_node(
                scrutinee,
                token_stream,
                context,
                type_interner,
                warnings,
                string_table,
            )?;
            return Ok(vec![match_statement]);
        }
        ParsedIfHeader::BoolCondition { condition } => condition,
    };

    ast_log!("Creating If Statement");
    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                token_stream.current_location(),
            ),
        ));
    }

    token_stream.advance();

    // Both bodies publish provisional generic requests into the shared sink. Finalisation owns
    // selection, so retain the exact parser boundaries without rescanning calls later.
    let then_request_start = context.generic_request_checkpoint();

    let then_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    let then_scope = then_context.scope.clone();
    let then_block = function_body_to_ast(
        token_stream,
        then_context,
        type_interner,
        warnings,
        string_table,
    )?;
    let then_request_end = context.generic_request_checkpoint();

    let (else_block, else_scope) = if token_stream.current_token_kind() == &TokenKind::Else {
        reject_same_line_else_if(token_stream)?;
        token_stream.advance();
        let else_context = context.new_child_control_flow(ContextKind::Branch, string_table);
        let else_scope = else_context.scope.clone();
        (
            Some(function_body_to_ast(
                token_stream,
                else_context,
                type_interner,
                warnings,
                string_table,
            )?),
            Some(else_scope),
        )
    } else {
        (None, None)
    };
    let else_request_end = context.generic_request_checkpoint();

    let request_ranges = IfGenericRequestRanges {
        then_branch: GenericRequestRange::new(then_request_start, then_request_end),
        else_branch: GenericRequestRange::new(then_request_end, else_request_end),
    };

    #[cfg(feature = "benchmark_counters")]
    add_ast_counter(
        AstCounter::BranchLocalGenericRequests,
        request_ranges.then_branch.len() + request_ranges.else_branch.len(),
    );

    Ok(vec![AstNode {
        kind: NodeKind::If(
            condition,
            then_block,
            else_block,
            IfBranchMetadata::new(request_ranges, then_scope.clone(), else_scope),
        ),
        location: token_stream.current_location(),
        scope: then_scope,
    }])
}

fn create_option_present_capture_branch(
    parsed_header: OptionPresentCaptureBranch,
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> BranchingResult<Vec<AstNode>> {
    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                token_stream.current_location(),
            ),
        ));
    }
    token_stream.advance();

    let body = function_body_to_ast(
        token_stream,
        parsed_header
            .then_context
            .new_child_control_flow(ContextKind::Branch, string_table),
        type_interner,
        warnings,
        string_table,
    )?;

    let else_block = if token_stream.current_token_kind() == &TokenKind::Else {
        reject_same_line_else_if(token_stream)?;
        token_stream.advance();
        let else_context = context.new_child_control_flow(ContextKind::Branch, string_table);
        Some(function_body_to_ast(
            token_stream,
            else_context,
            type_interner,
            warnings,
            string_table,
        )?)
    } else {
        None
    };

    // Single-predicate option match without `else` gets an implicit empty default
    // so the `none` case silently falls through, consistent with `if` statement semantics.
    let (default, exhaustiveness) = if let Some(else_body) = else_block {
        (Some(else_body), MatchExhaustiveness::HasDefault)
    } else {
        (Some(vec![]), MatchExhaustiveness::HasDefault)
    };

    Ok(vec![AstNode {
        kind: NodeKind::Match {
            scrutinee: parsed_header.scrutinee,
            arms: vec![MatchArm {
                pattern: parsed_header.pattern,
                guard: None,
                body,
            }],
            default,
            exhaustiveness,
        },
        location: token_stream.current_location(),
        scope: context.scope.clone(),
    }])
}

/// Parse a complete `if <subject> is:` match statement into a `NodeKind::Match`.
///
/// WHAT: loops through pattern/`else` arms, validates ordering and uniqueness, then
/// delegates exhaustiveness checking before returning the match node.
/// WHY: all match-level invariants (at least one pattern arm before else, no duplicates,
/// exhaustiveness) are enforced here so downstream HIR lowering can assume valid input.
fn create_match_node(
    scrutinee: Expression,
    token_stream: &mut FileTokens,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> BranchingResult<AstNode> {
    let parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        warnings,
        None,
        string_table,
    )?;

    Ok(AstNode {
        kind: NodeKind::Match {
            scrutinee: parsed_match.scrutinee,
            arms: parsed_match.arms,
            default: parsed_match.default,
            exhaustiveness: parsed_match.exhaustiveness,
        },
        location: parsed_match.location,
        scope: parsed_match.scope,
    })
}

/// Parse the shared contents of a full `if <subject> is:` match block.
///
/// WHAT: validates arm syntax, captures, guards, duplicate/unreachable arms, and
/// exhaustiveness, then returns the parsed match payload.
/// WHY: statement matches and value-producing matches have identical pattern
/// semantics; the value form only changes the active value target while parsing
/// arm bodies.
pub(crate) fn parse_match_block(
    scrutinee: Expression,
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    active_value_target: Option<ActiveValueProductionTarget>,
    string_table: &mut StringTable,
) -> BranchingResult<ParsedMatchBlock> {
    ast_log!("Creating Match Statement");

    if token_stream.current_token_kind() != &TokenKind::Colon {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                token_stream.current_location(),
            ),
        ));
    }

    token_stream.advance();
    let mut match_context = context.new_child_control_flow(ContextKind::Branch, string_table);
    match_context.active_value_target = active_value_target;

    let mut arms: Vec<MatchArm> = Vec::new();
    let mut else_block = None;
    let mut seen_else = false;
    let mut match_arm_indent: Option<i32> = None;
    let mut coverage_tracker = MatchArmCoverageTracker::default();

    // ----------------------------
    //  Parse match arms
    // ----------------------------
    loop {
        token_stream.skip_newlines();

        match token_stream.current_token_kind() {
            TokenKind::End => {
                let next_token = peek_next_non_newline_token(token_stream);
                let next_index = peek_next_non_newline_token_index(token_stream);
                let semicolon_separates_same_level_arms =
                    match (match_arm_indent, next_token, next_index) {
                        (Some(arm_indent), Some(next), Some(idx))
                            if next.kind == TokenKind::Else
                                || token_index_has_top_level_fat_arrow(token_stream, idx) =>
                        {
                            next.location.start_pos.char_column == arm_indent
                        }
                        _ => false,
                    };

                if semicolon_separates_same_level_arms {
                    return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                        InvalidMatchArmReason::SemicolonDelimiter,
                        token_stream.current_location(),
                    )));
                }
                token_stream.advance();
                break;
            }

            TokenKind::Eof => {
                return Err(branching_error(
                    CompilerDiagnostic::invalid_control_flow_statement(
                        InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch,
                        token_stream.current_location(),
                    ),
                ));
            }

            TokenKind::Else => {
                match_arm_indent
                    .get_or_insert(token_stream.current_location().start_pos.char_column);

                if arms.is_empty() {
                    return Err(branching_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::CaseRequiredBeforeElse,
                            token_stream.current_location(),
                        ),
                    ));
                }

                if seen_else {
                    return Err(branching_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::DuplicateElseArm,
                            token_stream.current_location(),
                        ),
                    ));
                }
                seen_else = true;

                else_block = Some(parse_else_arm(
                    token_stream,
                    &match_context,
                    type_interner,
                    warnings,
                    string_table,
                )?);
            }

            // Normal pattern arm or malformed header — delegate to dedicated parsing.
            _ => {
                if let Some(candidate) = current_token_starts_match_arm_header(token_stream) {
                    debug_assert_eq!(candidate.start_index, token_stream.index);
                    debug_assert!(candidate.arrow_index > candidate.start_index);
                    match_arm_indent.get_or_insert(candidate.start_location.start_pos.char_column);

                    let parsed = parse_match_arm(
                        &scrutinee,
                        token_stream,
                        &match_context,
                        type_interner,
                        warnings,
                        string_table,
                    )?;

                    if seen_else {
                        warnings.push(CompilerDiagnostic::unreachable_match_arm(
                            parsed.pattern_location.clone(),
                        ));
                    } else {
                        let coverage = coverage_tracker.record_arm(
                            &parsed.arm.pattern,
                            parsed.arm.guard.as_ref(),
                            parsed.matched_choice_variant,
                        );

                        if coverage.unreachable {
                            warnings.push(CompilerDiagnostic::unreachable_match_arm(
                                parsed.pattern_location.clone(),
                            ));
                        }
                    }
                    arms.push(parsed.arm);
                    continue;
                }

                if token_is_line_initial(token_stream, token_stream.index)
                    && current_line_contains_top_level_colon(token_stream)
                {
                    return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                        InvalidMatchArmReason::LegacyColonSyntax,
                        token_stream.current_location(),
                    )));
                }
                return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                    InvalidMatchArmReason::ExpectedArmHeader,
                    token_stream.current_location(),
                )));
            }
        }
    }

    // ----------------------------
    //  Enforce exhaustiveness and build node
    // ----------------------------
    enforce_match_exhaustiveness(MatchExhaustivenessCheck {
        scrutinee: &scrutinee,
        has_default: else_block.is_some(),
        facts: coverage_tracker.facts(),
        type_environment: type_interner.environment(),
    })?;

    let exhaustiveness = if else_block.is_some() {
        MatchExhaustiveness::HasDefault
    } else {
        MatchExhaustiveness::ExhaustiveChoice
    };

    Ok(ParsedMatchBlock {
        arms,
        default: else_block,
        exhaustiveness,
        location: token_stream.current_location(),
        scope: match_context.scope,
        scrutinee,
    })
}

fn parse_else_arm(
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> BranchingResult<Vec<AstNode>> {
    token_stream.advance();
    token_stream.skip_newlines();

    if token_stream.current_token_kind() == &TokenKind::Colon {
        return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::LegacyElseSyntax,
            token_stream.current_location(),
        )));
    }

    if token_stream.current_token_kind() == &TokenKind::Arrow {
        return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::InvalidArrow,
            token_stream.current_location(),
        )));
    }

    if token_stream.current_token_kind() != &TokenKind::FatArrow {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedFatArrow,
                token_stream.current_location(),
            ),
        ));
    }

    token_stream.advance();
    let arm_context = new_match_arm_body_context(match_context, string_table);
    function_body_to_ast(
        token_stream,
        arm_context,
        type_interner,
        warnings,
        string_table,
    )
}

/// Parse a single `<pattern> => <body>` arm.
///
/// WHAT: delegates reusable pattern/guard parsing to `match_headers`, then
/// validates the statement `=>` separator and parses the arm body.
/// WHY: `branching.rs` stays the owner of statement body parsing while template
/// match work can reuse the same header parser without entering this path.
///
/// ENTRY INVARIANT: the token stream is already positioned at the first token of the
/// pattern; this function does not advance before parsing.
fn parse_match_arm(
    scrutinee: &Expression,
    token_stream: &mut FileTokens,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    warnings: &mut Vec<CompilerDiagnostic>,
    string_table: &mut StringTable,
) -> BranchingResult<ParsedMatchArm> {
    let ParsedMatchArmHeader {
        pattern,
        guard,
        arm_scope,
        matched_choice_variant,
        pattern_location,
    } = parse_match_arm_header(
        scrutinee,
        token_stream,
        match_context,
        type_interner,
        &[TokenKind::FatArrow],
        string_table,
    )?;

    if token_stream.current_token_kind() != &TokenKind::FatArrow {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedFatArrow,
                token_stream.current_location(),
            ),
        ));
    }

    token_stream.advance();
    let arm_body_context = new_match_arm_body_context(&arm_scope, string_table);
    let body = function_body_to_ast(
        token_stream,
        arm_body_context,
        type_interner,
        warnings,
        string_table,
    )?;

    Ok(ParsedMatchArm {
        arm: MatchArm {
            pattern,
            guard,
            body,
        },
        matched_choice_variant,
        pattern_location,
    })
}

fn new_match_arm_body_context(
    match_context: &ScopeContext,
    string_table: &mut StringTable,
) -> ScopeContext {
    let active_value_target = match_context.active_value_target.clone();
    let mut arm_context = match_context.new_child_control_flow(ContextKind::MatchArm, string_table);
    arm_context.active_value_target = active_value_target;
    arm_context
}

#[cfg(test)]
#[path = "tests/branching_tests.rs"]
mod branching_tests;
