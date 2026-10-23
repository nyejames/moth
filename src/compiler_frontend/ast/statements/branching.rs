//! Match and if/else branching AST construction.
//!
//! WHAT: parses `if`/`else` conditionals and `if value is:` match statements into AST branch and
//! match nodes, or retains the exact authored pieces as numeric-dependent constructs while a
//! subject, pattern, guard or nested body still awaits the body root's selected types.
//! WHY: this module owns statement body parsing and final `NodeKind::Match` construction, while
//! shared match helpers provide reusable header parsing and exhaustiveness validation for later
//! template control-flow work.

use crate::ast_log;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, IfBranchMetadata, MatchExhaustiveness, NodeKind,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, OwnedExpr,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::numeric_construction::{
    BodyDraft, DeferredArmPattern, DeferredBody, DeferredConstruct, DeferredExpressionCompletion,
    DeferredIf, DeferredMatch, DeferredMatchArm, NumericContinuation, StatementExpression,
};
use crate::compiler_frontend::ast::statements::body_dispatch::{
    ParsedBody, parse_function_body_statements,
};
use crate::compiler_frontend::ast::statements::condition_validation::ensure_match_guard_condition;
use crate::compiler_frontend::ast::statements::if_headers::{ParsedIfHeader, parse_if_header};
use crate::compiler_frontend::ast::statements::match_arm_boundaries::{
    current_line_contains_top_level_colon, current_token_starts_match_arm_header,
    token_index_has_top_level_fat_arrow, token_is_line_initial,
};
use crate::compiler_frontend::ast::statements::match_exhaustiveness::{
    MatchArmCoverageTracker, MatchExhaustivenessCheck, MatchExhaustivenessSubject,
    enforce_match_exhaustiveness,
};
use crate::compiler_frontend::ast::statements::match_headers::{
    ParsedMatchArmHeader, parse_match_arm_header, retained_match_subject,
};
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchArm, MatchSubject, ensure_pattern_compatible_with_subject, finish_pending_choice_pattern,
    finish_pending_option_pattern, finish_retained_pattern,
};
use crate::compiler_frontend::ast::statements::value_production::types::ActiveValueProductionTarget;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, InvalidControlFlowStatementReason, InvalidMatchArmReason,
    ValueReceiverContext,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;

use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;

/// Branches recursively parse function bodies, so retained-data failures travel to the module
/// emission boundary instead of being recast as authored control-flow diagnostics.
type BranchingResult<T> = Result<T, ExpressionParseError>;

fn branching_error(diagnostic: CompilerDiagnostic) -> ExpressionParseError {
    diagnostic.into()
}

/// One parsed nested body kept in authored order until its construct publishes or retains it.
///
/// WHAT: the authored draft plus the scope the body was parsed under.
/// WHY: a nested body shares the enclosing function's construction domain, so its numeric-dependent
///      statements cannot complete here; keeping the draft (instead of early nodes) is what lets
///      `if`/`else` and match arms hand the exact authored order to body-root completion.
struct MatchBody {
    draft: BodyDraft,
    scope: PathId,
}

impl MatchBody {
    fn from_parsed(parsed: ParsedBody, scope: PathId) -> Self {
        match parsed {
            // A nested body never completes as an independent root; wrapping finished nodes keeps
            // the one representation even if a caller ever hands back a completed body.
            ParsedBody::Complete(nodes) => {
                let mut draft = BodyDraft::new();
                draft.extend_nodes(nodes);
                Self { draft, scope }
            }
            ParsedBody::Draft(draft) => Self { draft, scope },
        }
    }

    /// An authored-empty body (the implicit `else` of a single-predicate option capture).
    fn empty(scope: PathId) -> Self {
        Self {
            draft: BodyDraft::new(),
            scope,
        }
    }

    fn has_pending(&self) -> bool {
        self.draft.has_pending()
    }

    fn into_deferred(self) -> DeferredBody {
        DeferredBody {
            draft: self.draft,
            scope: self.scope,
        }
    }

    fn into_nodes(self, position: &str) -> BranchingResult<Vec<AstNode>> {
        self.draft.into_nodes_for_host(position)
    }
}

/// Intermediate result of parsing a single match arm.
struct ParsedMatchArm {
    pattern: DeferredArmPattern,
    guard: Option<StatementExpression>,
    body: MatchBody,
    /// Tracks which choice variant this arm consumes so duplicates can be rejected early.
    matched_choice_variant: Option<StringId>,
    pattern_span: Option<SourceSpan>,
}

impl ParsedMatchArm {
    /// True when the pattern, guard and body all finished at the parse position.
    fn is_ready(&self) -> bool {
        matches!(self.pattern, DeferredArmPattern::Ready(_))
            && !matches!(self.guard, Some(StatementExpression::Retained(_)))
            && !self.body.has_pending()
    }

    /// Build the concrete arm of a fully materialised match.
    fn into_ready(self, position: &str) -> BranchingResult<MatchArm> {
        let DeferredArmPattern::Ready(pattern) = self.pattern else {
            return Err(CompilerError::compiler_error(
                "a retained match pattern reached the concrete arm construction",
            )
            .into());
        };
        let guard = match self.guard {
            None => None,
            Some(StatementExpression::Built(guard)) => Some(guard),
            Some(StatementExpression::Retained(_)) => {
                return Err(CompilerError::compiler_error(
                    "a retained match guard reached the concrete arm construction",
                )
                .into());
            }
        };
        Ok(MatchArm {
            pattern,
            guard,
            body: self.body.into_nodes(position)?,
        })
    }

    /// Move the arm into the retained match representation.
    fn into_deferred(self) -> DeferredMatchArm {
        DeferredMatchArm {
            pattern: self.pattern,
            guard: self.guard,
            body: self.body.into_deferred(),
        }
    }
}

struct OptionPresentCaptureBranch {
    scrutinee: StatementExpression,
    pattern: DeferredArmPattern,
    then_context: ScopeContext,
    header_span: Option<SourceSpan>,
}

/// Parsed match block payload shared by statement and value-producing matches.
///
/// WHAT: either the fully materialised match (subject expression, ready arms and bodies) or the
/// retained record that resumes at the body root.
/// WHY: value-producing matches must reuse statement match parsing rules without manufacturing a
/// temporary statement node, and a retained match must carry exactly the authored pieces the
/// construct lane completes.
#[allow(
    clippy::large_enum_variant,
    reason = "the ready payload is the authored match result; the retained variant is already boxed, and boxing the ready payload too would only add indirection to the common path"
)]
pub(crate) enum ParsedMatchBlock {
    /// The subject, every arm pattern, guard and body finished at the parse position.
    Ready(ReadyMatchBlock),
    /// At least one position is retained; the record resumes at the enclosing body root.
    Retained(Box<DeferredMatch>),
}

/// One fully materialised match block.
pub(crate) struct ReadyMatchBlock {
    pub scrutinee: ExprId,
    pub arms: Vec<MatchArm>,
    pub default: Option<Vec<AstNode>>,
    pub exhaustiveness: MatchExhaustiveness,
    pub scope: PathId,
}

/// One retained match resumed through the completion session.
///
/// WHAT: the resolved subject, arms, default and exhaustiveness contract a completion lane needs
/// after every authored position has materialised at the subject's selected type.
/// WHY: the statement drain and the value-producing match lane must not each re-derive pattern
/// materialisation, guard validation and body resolution.
pub(crate) struct ResumedMatchBlock {
    pub(crate) scrutinee: ExprId,
    pub(crate) arms: Vec<MatchArm>,
    pub(crate) default: Option<Vec<AstNode>>,
    pub(crate) exhaustiveness: MatchExhaustiveness,
    pub(crate) span: Option<SourceSpan>,
}
/// Which receiving contract applies to bodies of a retained match.
///
/// Statement matches inherit their enclosing handler, while a value-producing match resolves its
/// arms under the value block's own contract, including an explicit inferred-contract mask.
#[derive(Clone, Copy)]
pub(crate) enum DeferredMatchBodyReceiver<'a> {
    Inherit,
    Own(Option<&'a [crate::compiler_frontend::datatypes::ids::TypeId]>),
}

/// Peek at the index and tag of the next non-newline token without advancing.
///
/// Restores the entry position before returning, including the not-found path.
fn peek_next_non_newline(token_stream: &mut AstCursor) -> Option<(usize, TokenTag)> {
    let resume = token_stream.position();
    token_stream.advance();
    let mut found = None;
    while !token_stream.is_at_end() {
        if token_stream.current_tag() != TokenTag::NEWLINE {
            found = Some((token_stream.position(), token_stream.current_tag()));
            break;
        }
        token_stream.advance();
    }
    token_stream
        .set_position(resume)
        .expect("peek resume stays inside the active parser view");
    found
}

fn reject_same_line_else_if(token_stream: &AstCursor) -> BranchingResult<()> {
    let else_span = Some(token_stream.current_span());
    let Some(next_tag) = token_stream.peek_next_tag() else {
        return Ok(());
    };

    // Statement `else if` is deliberately not a branch-chain syntax. A nested
    // `if` remains available as the first statement inside a separate `else` body.
    if next_tag == TokenTag::IF {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ElseIfUnsupported,
                else_span,
            ),
        ));
    }

    Ok(())
}

/// Parse an `if` statement or an `if <subject> is:` match statement.
///
/// WHAT: detects single-predicate option matches (`if option is |name|:`) before falling back to
/// normal `if`/`else` parsing or statement-style match parsing, and pushes either the finished
/// node or one retained construct into the caller's body draft.
/// WHY: the single-predicate and statement-match shapes share the `if` keyword, so this entry
/// point disambiguates them early based on token lookahead; a retained condition, body or nested
/// statement keeps the authored pieces until the body root completes them.
#[allow(
    clippy::too_many_arguments,
    reason = "if statement construction keeps the token stream, scope, mutable interner/builder/constant/warning/string/path state and the caller's body draft as separate borrows"
)]
pub fn create_branch(
    token_stream: &mut AstCursor,
    context: ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    draft: &mut BodyDraft,
) -> BranchingResult<()> {
    let header_span = token_stream
        .previous_span()
        .or_else(|| Some(token_stream.current_span()));
    let parsed_header = parse_if_header(
        token_stream,
        &context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
    )?;

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
                    header_span,
                },
                token_stream,
                &context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
                draft,
            );
        }
        ParsedIfHeader::MatchStyle { scrutinee } => {
            return create_match_node(
                scrutinee,
                header_span,
                token_stream,
                &context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
                draft,
            );
        }
        ParsedIfHeader::BoolCondition { condition } => condition,
    };

    ast_log!("Creating If Statement");
    if token_stream.current_tag() != TokenTag::COLON {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                Some(token_stream.current_span()),
            ),
        ));
    }
    token_stream.advance();

    // Both bodies publish provisional generic requests into the shared sink. Finalisation owns
    // selection, so retain the exact parser boundaries without rescanning calls later.
    let then_request_start = context.generic_request_checkpoint();
    let then_context = context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
    let then_scope = then_context.scope;
    let then_body = MatchBody::from_parsed(
        parse_function_body_statements(
            token_stream,
            then_context,
            type_interner,
            expression_builder,
            const_values,
            warnings,
            string_table,
            path_fork,
        )?,
        then_scope,
    );
    let then_request_end = context.generic_request_checkpoint();

    let (else_body, else_scope) = if token_stream.current_tag() == TokenTag::ELSE {
        reject_same_line_else_if(token_stream)?;
        token_stream.advance();
        let else_context =
            context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
        let else_scope = else_context.scope;
        let else_body = MatchBody::from_parsed(
            parse_function_body_statements(
                token_stream,
                else_context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
            )?,
            else_scope,
        );
        (Some(else_body), Some(else_scope))
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

    if matches!(&condition, StatementExpression::Retained(_))
        || then_body.has_pending()
        || else_body.as_ref().is_some_and(MatchBody::has_pending)
    {
        let deferred = DeferredIf {
            condition,
            then_body: then_body.into_deferred(),
            else_body: else_body.map(MatchBody::into_deferred),
            span: header_span,
            scope: then_scope,
            then_scope,
            else_scope,
            generic_request_start: then_request_start,
            then_request_end,
            else_request_end,
        };
        let continuation = context.register_numeric_continuation(
            NumericContinuation::Construct(Box::new(DeferredConstruct::If(Box::new(deferred)))),
        )?;
        draft.push_pending(continuation);
        return Ok(());
    }

    let StatementExpression::Built(condition) = condition else {
        return Err(CompilerError::compiler_error(
            "an if condition was retained after the branch decided it needs no numeric completion",
        )
        .into());
    };
    let then_block = then_body.into_nodes("an if then branch")?;
    let else_block = match else_body {
        Some(body) => Some(body.into_nodes("an if else branch")?),
        None => None,
    };

    draft.push_node(AstNode {
        kind: NodeKind::If(
            condition,
            then_block,
            else_block,
            IfBranchMetadata::new(request_ranges, then_scope, else_scope),
        ),
        span: header_span,
        scope: then_scope,
    });
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "single-predicate capture construction keeps the parsed header, token stream, scope, mutable interner/builder/constant/warning/string/path state and the caller's body draft as separate borrows"
)]
fn create_option_present_capture_branch(
    parsed_header: OptionPresentCaptureBranch,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    draft: &mut BodyDraft,
) -> BranchingResult<()> {
    let OptionPresentCaptureBranch {
        scrutinee,
        pattern,
        then_context,
        header_span,
    } = parsed_header;

    if token_stream.current_tag() != TokenTag::COLON {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                Some(token_stream.current_span()),
            ),
        ));
    }
    token_stream.advance();
    let body_context =
        then_context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
    let body_scope = body_context.scope;
    let body = MatchBody::from_parsed(
        parse_function_body_statements(
            token_stream,
            body_context,
            type_interner,
            expression_builder,
            const_values,
            warnings,
            string_table,
            path_fork,
        )?,
        body_scope,
    );

    let else_body = if token_stream.current_tag() == TokenTag::ELSE {
        reject_same_line_else_if(token_stream)?;
        token_stream.advance();
        let else_context =
            context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
        let else_scope = else_context.scope;
        Some(MatchBody::from_parsed(
            parse_function_body_statements(
                token_stream,
                else_context,
                type_interner,
                expression_builder,
                const_values,
                warnings,
                string_table,
                path_fork,
            )?,
            else_scope,
        ))
    } else {
        None
    };

    // Single-predicate option match without `else` gets an implicit empty default
    // so the `none` case silently falls through, consistent with `if` statement semantics.
    let default_body = else_body.unwrap_or_else(|| MatchBody::empty(context.scope));
    let has_pending = body.has_pending() || default_body.has_pending();

    match (scrutinee, pattern) {
        (StatementExpression::Built(scrutinee), DeferredArmPattern::Ready(pattern))
            if !has_pending =>
        {
            let body = body.into_nodes("a single-predicate capture then branch")?;
            let default = default_body.into_nodes("a single-predicate capture else branch")?;

            draft.push_node(AstNode {
                kind: NodeKind::Match {
                    scrutinee,
                    arms: vec![MatchArm {
                        pattern,
                        guard: None,
                        body,
                    }],
                    default: Some(default),
                    exhaustiveness: MatchExhaustiveness::HasDefault,
                },
                span: header_span,
                scope: context.scope,
            });
            Ok(())
        }
        (scrutinee, pattern) => push_deferred_option_capture(
            scrutinee,
            pattern,
            body,
            default_body,
            header_span,
            context,
            draft,
        ),
    }
}

/// Registers a statement option capture as one deferred match continuation.
///
/// WHAT: keeps the authored subject, pattern and bodies together until the body root selects
/// the numeric origin they reference.
/// WHY: a retained subject, a pending pattern or a pending body has no selected type here, so the
/// capture cannot publish a `NodeKind::Match` yet.
fn push_deferred_option_capture(
    scrutinee: StatementExpression,
    pattern: DeferredArmPattern,
    body: MatchBody,
    default_body: MatchBody,
    header_span: Option<SourceSpan>,
    context: &ScopeContext,
    draft: &mut BodyDraft,
) -> BranchingResult<()> {
    let deferred = DeferredMatch {
        scrutinee,
        arms: vec![DeferredMatchArm {
            pattern,
            guard: None,
            body: body.into_deferred(),
        }],
        default: Some(default_body.into_deferred()),
        exhaustiveness: MatchExhaustiveness::HasDefault,
        span: header_span,
        scope: context.scope,
        // The default follows the single authored arm, so no arm sits after the boundary.
        else_arm_index: Some(1),
    };
    let continuation = context.register_numeric_continuation(NumericContinuation::Construct(
        Box::new(DeferredConstruct::Match(Box::new(deferred))),
    ))?;
    draft.push_pending(continuation);
    Ok(())
}

/// Parse a complete `if <subject> is:` match statement into a draft position.
///
/// WHAT: delegates the shared block parse and pushes either the finished `NodeKind::Match` or one
/// retained construct into the caller's body draft.
/// WHY: a statement match whose subject, pattern, guard or body still awaits the body root keeps
/// the authored pieces instead of publishing nodes with unresolved numeric origins.
#[allow(
    clippy::too_many_arguments,
    reason = "match node construction keeps the subject, header span, token stream, scope, mutable interner/builder/constant/warning/string/path state and the caller's body draft as separate borrows"
)]
fn create_match_node(
    scrutinee: StatementExpression,
    header_span: Option<SourceSpan>,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    draft: &mut BodyDraft,
) -> BranchingResult<()> {
    let parsed_match = parse_match_block(
        scrutinee,
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        warnings,
        None,
        string_table,
        path_fork,
    )?;

    match parsed_match {
        ParsedMatchBlock::Ready(ready) => {
            draft.push_node(AstNode {
                kind: NodeKind::Match {
                    scrutinee: ready.scrutinee,
                    arms: ready.arms,
                    default: ready.default,
                    exhaustiveness: ready.exhaustiveness,
                },
                span: header_span,
                scope: ready.scope,
            });
        }
        ParsedMatchBlock::Retained(mut deferred) => {
            deferred.span = header_span;
            let continuation = context.register_numeric_continuation(
                NumericContinuation::Construct(Box::new(DeferredConstruct::Match(deferred))),
            )?;
            draft.push_pending(continuation);
        }
    }

    Ok(())
}

/// Parse the shared contents of a full `if <subject> is:` match block.
///
/// WHAT: validates arm syntax, captures, guards, duplicate/unreachable arms, and exhaustiveness,
/// then returns either the fully materialised payload or the retained record.
/// WHY: statement matches and value-producing matches have identical pattern
/// semantics; the value form only changes the active value target while parsing
/// arm bodies.
#[allow(
    clippy::too_many_arguments,
    reason = "match block parsing keeps the subject, token stream, scope, mutable interner/builder/constant/warning/string/path state, and optional value target as separate borrows"
)]
pub(crate) fn parse_match_block(
    scrutinee: StatementExpression,
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    active_value_target: Option<ActiveValueProductionTarget>,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> BranchingResult<ParsedMatchBlock> {
    ast_log!("Creating Match Statement");

    if token_stream.current_tag() != TokenTag::COLON {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedColonAfterCondition,
                Some(token_stream.current_span()),
            ),
        ));
    }

    token_stream.advance();
    let mut match_context =
        context.new_child_control_flow(ContextKind::Branch, string_table, path_fork);
    if let Some(active_value_target) = active_value_target {
        match_context.active_value_target = Some(active_value_target);
    }

    // A retained subject classifies through the shared retained operand owner: a concrete type
    // names its subject type, and a retained generic choice names its variants from its declared
    // owner so its arms parse before the subject resolves. Any other pending numeric subject
    // materialises its patterns once the selected type exists.
    let subject = match &scrutinee {
        StatementExpression::Built(scrutinee) => MatchSubject::Known(
            expression_builder
                .view(*scrutinee)?
                .require_single(ValueReceiverContext::MatchScrutinee)?,
        ),
        StatementExpression::Retained(retained) => retained_match_subject(
            &match_context,
            type_interner.environment(),
            &retained.items,
            expression_builder,
        ),
    };

    let mut arms: Vec<ParsedMatchArm> = Vec::new();
    let mut else_block: Option<MatchBody> = None;
    // The authored arm index of the `else` arm, retained so completion can reproduce the
    // unreachable-arm policy for exactly the arms parse did not already report.
    let mut else_arm_index: Option<usize> = None;
    let mut seen_else = false;
    let mut coverage_tracker = MatchArmCoverageTracker::default();

    // ----------------------------
    //  Parse match arms
    // ----------------------------
    loop {
        token_stream.skip_newlines();

        match token_stream.current_tag() {
            TokenTag::END => {
                let next = peek_next_non_newline(token_stream);
                let semicolon_separates_same_level_arms = !seen_else
                    && next.is_some_and(|(index, tag)| {
                        tag == TokenTag::ELSE
                            || token_index_has_top_level_fat_arrow(token_stream, index)
                    });

                if semicolon_separates_same_level_arms {
                    return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                        InvalidMatchArmReason::SemicolonDelimiter,
                        Some(token_stream.current_span()),
                    )));
                }
                token_stream.advance();
                break;
            }

            TokenTag::EOF => {
                return Err(branching_error(
                    CompilerDiagnostic::invalid_control_flow_statement(
                        InvalidControlFlowStatementReason::UnexpectedEndOfFileInMatch,
                        Some(token_stream.current_span()),
                    ),
                ));
            }

            TokenTag::ELSE => {
                if arms.is_empty() {
                    return Err(branching_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::CaseRequiredBeforeElse,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }

                if seen_else {
                    return Err(branching_error(
                        CompilerDiagnostic::invalid_control_flow_statement(
                            InvalidControlFlowStatementReason::DuplicateElseArm,
                            Some(token_stream.current_span()),
                        ),
                    ));
                }
                seen_else = true;
                else_arm_index = Some(arms.len());

                else_block = Some(parse_else_arm(
                    token_stream,
                    &match_context,
                    type_interner,
                    expression_builder,
                    const_values,
                    warnings,
                    string_table,
                    path_fork,
                )?);
            }

            // Normal pattern arm or malformed header — delegate to dedicated parsing.
            _ => {
                if let Some(candidate) = current_token_starts_match_arm_header(token_stream) {
                    debug_assert_eq!(candidate.start_index, token_stream.position());
                    debug_assert!(candidate.arrow_index > candidate.start_index);
                    let parsed = parse_match_arm(
                        &subject,
                        token_stream,
                        &match_context,
                        type_interner,
                        expression_builder,
                        const_values,
                        warnings,
                        string_table,
                        path_fork,
                    )?;

                    if seen_else {
                        warnings.push(CompilerDiagnostic::unreachable_match_arm(
                            parsed.pattern_span,
                        ));
                    } else {
                        let coverage = coverage_tracker.record_deferred_arm(
                            &parsed.pattern,
                            parsed.guard.is_some(),
                            parsed.matched_choice_variant,
                            const_values,
                        );

                        if coverage.unreachable {
                            warnings.push(CompilerDiagnostic::unreachable_match_arm(
                                parsed.pattern_span,
                            ));
                        }
                    }
                    arms.push(parsed);
                    continue;
                }

                if token_is_line_initial(token_stream, token_stream.position())
                    && current_line_contains_top_level_colon(token_stream)
                {
                    return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                        InvalidMatchArmReason::LegacyColonSyntax,
                        Some(token_stream.current_span()),
                    )));
                }
                return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
                    InvalidMatchArmReason::ExpectedArmHeader,
                    Some(token_stream.current_span()),
                )));
            }
        }
    }

    // ----------------------------
    //  Enforce exhaustiveness and build the payload
    // ----------------------------
    let exhaustiveness_subject = match (&scrutinee, subject) {
        (StatementExpression::Built(scrutinee), _) => {
            let scrutinee_view = expression_builder.view(*scrutinee)?;
            MatchExhaustivenessSubject::Known {
                type_id: scrutinee_view.value_type_id()?,
                span: scrutinee_view.span(),
            }
        }
        (StatementExpression::Retained(fragment), MatchSubject::Known(type_id)) => {
            MatchExhaustivenessSubject::Known {
                type_id,
                span: fragment.span,
            }
        }
        (StatementExpression::Retained(fragment), MatchSubject::PendingChoice(declared)) => {
            MatchExhaustivenessSubject::PendingChoice {
                declared_type_id: declared.declared_type_id,
                span: fragment.span,
            }
        }
        (StatementExpression::Retained(fragment), MatchSubject::PendingOption { .. }) => {
            MatchExhaustivenessSubject::PendingOption {
                span: fragment.span,
            }
        }
        (StatementExpression::Retained(fragment), _) => {
            MatchExhaustivenessSubject::PendingNumeric {
                span: fragment.span,
            }
        }
    };
    enforce_match_exhaustiveness(MatchExhaustivenessCheck {
        subject: exhaustiveness_subject,
        has_default: else_block.is_some(),
        facts: coverage_tracker.facts(),
        type_environment: type_interner.environment(),
    })?;

    let exhaustiveness = if else_block.is_some() {
        MatchExhaustiveness::HasDefault
    } else {
        MatchExhaustiveness::ExhaustiveChoice
    };

    build_parsed_match_block(
        scrutinee,
        arms,
        else_block,
        else_arm_index,
        exhaustiveness,
        match_context.scope,
    )
}

/// Publish a parsed match block, or retain it while any authored position awaits completion.
///
/// WHAT: returns the concrete payload when the subject, every pattern, guard and body finished at
/// the parse position, and the retained record otherwise.
/// WHY: a retained subject, pattern, guard or body cannot produce a typed `MatchArm` yet, so the
/// whole authored match travels to the body root instead of publishing a partial node.
fn build_parsed_match_block(
    scrutinee: StatementExpression,
    arms: Vec<ParsedMatchArm>,
    else_block: Option<MatchBody>,
    else_arm_index: Option<usize>,
    exhaustiveness: MatchExhaustiveness,
    scope: PathId,
) -> BranchingResult<ParsedMatchBlock> {
    let ready = matches!(&scrutinee, StatementExpression::Built(_))
        && arms.iter().all(ParsedMatchArm::is_ready)
        && !else_block.as_ref().is_some_and(MatchBody::has_pending);

    if !ready {
        let arms = arms
            .into_iter()
            .map(ParsedMatchArm::into_deferred)
            .collect();
        let default = else_block.map(MatchBody::into_deferred);

        return Ok(ParsedMatchBlock::Retained(Box::new(DeferredMatch {
            scrutinee,
            arms,
            default,
            exhaustiveness,
            span: None,
            scope,
            else_arm_index,
        })));
    }

    let StatementExpression::Built(scrutinee) = scrutinee else {
        return Err(CompilerError::compiler_error(
            "a retained match subject reached the concrete match construction",
        )
        .into());
    };
    let arms = arms
        .into_iter()
        .map(|arm| arm.into_ready("a match arm body"))
        .collect::<BranchingResult<Vec<MatchArm>>>()?;
    let default = match else_block {
        Some(body) => Some(body.into_nodes("a match default body")?),
        None => None,
    };

    Ok(ParsedMatchBlock::Ready(ReadyMatchBlock {
        scrutinee,
        arms,
        default,
        exhaustiveness,
        scope,
    }))
}

/// Resume one retained match through the body-root completion session.
///
/// WHAT: selects the subject's exact type, materialises every retained pattern at that type,
///       completes retained guards, resolves every retained arm and default body in authored
///       order, and emits the duplicate-arm warnings that only materialisation can decide.
/// WHY: statement matches (through the body-completion drain) and value matches (through their
///      value-block driver) must not each re-derive arm materialisation, so the match producer
///      owns the one resume for the record it created; parse-time coverage cannot compare a
///      retained numeric pattern's value before the subject type exists.
pub(crate) fn resume_deferred_match<'env>(
    deferred: DeferredMatch,
    body_receiver: DeferredMatchBodyReceiver<'_>,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<ResumedMatchBlock, ExpressionParseError> {
    let DeferredMatch {
        scrutinee,
        arms,
        default,
        exhaustiveness,
        span,
        scope: _,
        else_arm_index,
    } = deferred;

    let scrutinee = completion.resolve_statement_expression(scrutinee)?;
    let subject_type = {
        let services = completion.session_services();
        let subject_view = services.expression_builder.view_owned(scrutinee)?;
        subject_view.require_single(ValueReceiverContext::MatchScrutinee)?
    };

    // Parse-time coverage already reported every pattern whose value existed then. A retained
    // pattern's exact scalar only exists after the subject selection above, so the duplicate rule
    // runs here over exactly those arms, in authored order. Arms at or after the authored `else`
    // were already reported unreachable while parsing and stay out of this pass so no arm is
    // reported twice.
    let mut resumed_coverage = MatchArmCoverageTracker::default();

    let mut resolved_arms = Vec::with_capacity(arms.len());
    for (arm_index, arm) in arms.into_iter().enumerate() {
        let DeferredMatchArm {
            pattern,
            guard,
            body,
        } = arm;
        let pattern_was_retained = matches!(&pattern, DeferredArmPattern::Retained(_));

        let pattern = match pattern {
            DeferredArmPattern::Ready(pattern) => {
                // A pattern authored before the subject resolved still needs its authored
                // compatibility check against the subject's selected exact type.
                let services = completion.session_services();
                ensure_pattern_compatible_with_subject(
                    &pattern,
                    subject_type,
                    services.type_interner.environment(),
                )?;
                pattern
            }
            DeferredArmPattern::PendingChoice(pending) => {
                // The subject instance is selected now, so each captured field takes its payload
                // type from that instance's variant view.
                let services = completion.session_services();
                finish_pending_choice_pattern(
                    pending,
                    subject_type,
                    services.type_interner.environment(),
                )?
            }
            DeferredArmPattern::PendingOption(pending) => {
                // The optional's inner type is selected now, so a retained value or capture takes
                // its numeric type from the selected subject.
                finish_pending_option_pattern(pending, subject_type, completion)?
            }
            DeferredArmPattern::Retained(retained) => {
                // A pattern value is typed by the match subject, which is only selected now.
                let value = completion.resolve_fragment(
                    retained.value.items,
                    ExpectedType::NumericLiteral(subject_type),
                    &retained.value.value_mode,
                )?;
                let services = completion.session_services();
                let value_view = services.expression_builder.view_owned(value)?;
                finish_retained_pattern(retained.relational, value_view)?
            }
        };

        let guard = match guard {
            Some(guard) => {
                let guard = completion.resolve_statement_expression(guard)?;
                let services = completion.session_services();
                let guard_view = services.expression_builder.view_owned(guard)?;
                ensure_match_guard_condition(guard_view, services.type_interner.environment())?;
                Some(guard)
            }
            None => None,
        };

        if pattern_was_retained && else_arm_index.is_none_or(|else_index| arm_index < else_index) {
            // Choice-variant dedupe and option facts were decided at parse; only the literal
            // duplicate rule needs the materialised value.
            let services = completion.session_services();
            let coverage =
                resumed_coverage.record_arm(&pattern, guard.is_some(), None, services.const_values);
            if coverage.unreachable {
                let services = completion.session_services();
                services
                    .context
                    .emit_warning(CompilerDiagnostic::unreachable_match_arm(pattern.span()));
            }
        }

        let body = match body_receiver {
            DeferredMatchBodyReceiver::Inherit => completion.resolve_deferred_body(body)?,
            DeferredMatchBodyReceiver::Own(expectations) => {
                completion.resolve_deferred_body_with_expectations(body, expectations)?
            }
        };
        resolved_arms.push(MatchArm {
            pattern,
            guard: guard.map(OwnedExpr::id),
            body,
        });
    }

    let default = match default {
        Some(body) => Some(match body_receiver {
            DeferredMatchBodyReceiver::Inherit => completion.resolve_deferred_body(body)?,
            DeferredMatchBodyReceiver::Own(expectations) => {
                completion.resolve_deferred_body_with_expectations(body, expectations)?
            }
        }),
        None => None,
    };

    Ok(ResumedMatchBlock {
        scrutinee: scrutinee.id(),
        arms: resolved_arms,
        default,
        exhaustiveness,
        span,
    })
}

#[allow(
    clippy::too_many_arguments,
    reason = "else arm parsing keeps the token stream, scope, mutable interner/builder/constant/warning/string/path state as separate borrows"
)]
fn parse_else_arm(
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> BranchingResult<MatchBody> {
    token_stream.advance();
    token_stream.skip_newlines();

    if token_stream.current_tag() == TokenTag::COLON {
        return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::LegacyElseSyntax,
            Some(token_stream.current_span()),
        )));
    }

    if token_stream.current_tag() == TokenTag::ARROW {
        return Err(branching_error(CompilerDiagnostic::invalid_match_arm(
            InvalidMatchArmReason::InvalidArrow,
            Some(token_stream.current_span()),
        )));
    }

    if token_stream.current_tag() != TokenTag::FAT_ARROW {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedFatArrow,
                Some(token_stream.current_span()),
            ),
        ));
    }

    token_stream.advance();
    let arm_context = new_match_arm_body_context(match_context, string_table, path_fork);
    let arm_scope = arm_context.scope;
    Ok(MatchBody::from_parsed(
        parse_function_body_statements(
            token_stream,
            arm_context,
            type_interner,
            expression_builder,
            const_values,
            warnings,
            string_table,
            path_fork,
        )?,
        arm_scope,
    ))
}

/// Parse a single `<pattern> => <body>` arm.
///
/// WHAT: delegates reusable pattern/guard parsing to `match_headers`, then validates the statement
/// `=>` separator and parses the arm body into its retained-capable representation.
/// WHY: `branching.rs` stays the owner of statement body parsing while template
/// match work can reuse the same header parser without entering this path.
///
/// ENTRY INVARIANT: the token stream is already positioned at the first token of the
/// pattern; this function does not advance before parsing.
#[allow(
    clippy::too_many_arguments,
    reason = "match arm parsing keeps the subject, token stream, scope, mutable interner/builder/constant/warning/string/path state as separate borrows"
)]
fn parse_match_arm(
    subject: &MatchSubject,
    token_stream: &mut AstCursor,
    match_context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    warnings: &mut DiagnosticBag,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> BranchingResult<ParsedMatchArm> {
    let ParsedMatchArmHeader {
        pattern,
        guard,
        arm_scope,
        matched_choice_variant,
        pattern_span,
    } = parse_match_arm_header(
        subject,
        token_stream,
        match_context,
        type_interner,
        expression_builder,
        const_values,
        &[TokenTag::FAT_ARROW],
        string_table,
        path_fork,
    )?;

    if token_stream.current_tag() != TokenTag::FAT_ARROW {
        return Err(branching_error(
            CompilerDiagnostic::invalid_control_flow_statement(
                InvalidControlFlowStatementReason::ExpectedFatArrow,
                Some(token_stream.current_span()),
            ),
        ));
    }

    token_stream.advance();
    let arm_body_context = new_match_arm_body_context(&arm_scope, string_table, path_fork);
    let arm_body_scope = arm_body_context.scope;
    let body = MatchBody::from_parsed(
        parse_function_body_statements(
            token_stream,
            arm_body_context,
            type_interner,
            expression_builder,
            const_values,
            warnings,
            string_table,
            path_fork,
        )?,
        arm_body_scope,
    );

    Ok(ParsedMatchArm {
        pattern,
        guard,
        body,
        matched_choice_variant,
        pattern_span,
    })
}

fn new_match_arm_body_context(
    match_context: &ScopeContext,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> ScopeContext {
    let active_value_target = match_context.active_value_target.clone();
    let mut arm_context =
        match_context.new_child_control_flow(ContextKind::MatchArm, string_table, path_fork);
    arm_context.active_value_target = active_value_target;
    arm_context
}

#[cfg(test)]
#[path = "tests/branching_tests.rs"]
mod branching_tests;
