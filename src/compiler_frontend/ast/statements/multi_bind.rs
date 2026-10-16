//! Multi-bind parsing and target resolution helpers.
//!
//! WHAT: parses `a, b = pair()` style statements and resolves each target into the AST shape that HIR
//! lowering consumes later.
//! WHY: multi-bind syntax mixes statement parsing, declaration rules, and target validation, so it
//! deserves a dedicated module instead of living inside the general function-body dispatcher.
//!
//! INVARIANT: multi-bind is a special-purpose surface for explicit multi-value-producing
//! expressions. It is NOT a generic destructuring mechanism. The right-hand side must belong to
//! an explicitly supported multi-bind-producing surface (currently: multi-return calls and
//! supported value-producing blocks). Future surfaces should extend that classifier, not broaden
//! regular declaration syntax.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, DeclarationValue, MultiBindTarget, MultiBindTargetKind, NodeKind,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, into_single_operand, single_operand,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::expressions::parse_expression::create_expression_with_trailing_newline_policy;
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::field_access::{RetainedTerminalCall, retained_terminal_call};
use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCall, DeferredExpression, DeferredExpressionSuffix, DeferredMultiBindTarget,
    DeferredMultiBindValue, MultiBindContinuation, MultiBindProducerFact, NumericConstructionState,
    NumericContinuation, NumericContinuationId, NumericRecipe, PendingMultiBindProducer,
    PendingMultiBindProducerKind, StatementExpression, binding_reference_shape,
    register_binding_witness, register_pending_reference_view, union_alias_classes,
    union_pending_binding_with_generic_slot,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShapeId, exact_shape_type, project_exact_shape, register_shape_witness,
    shapes_join_refused, union_shape_classes,
};
use crate::compiler_frontend::ast::statements::fallible_handling::DeferredCatchHandler;
use crate::compiler_frontend::ast::statements::value_production::try_parse_multi_bind_value_block;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    TypeResolutionContext, TypeResolutionContextInputs, resolve_parsed_type_annotation,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, InvalidControlFlowStatementReason,
    InvalidMultiBindReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::parse_binding_target_syntax;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, SourceSpan};
use crate::compiler_frontend::symbols::identifier_policy::{
    IdentifierNamingKind, ensure_not_keyword_shadow_identifier, naming_warning_for_identifier,
};
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::parse_context::{CastTargetContext, ExpectedType};
use crate::compiler_frontend::utilities::token_scan::has_top_level_comma_before_statement_end;
use crate::compiler_frontend::value_mode::ValueMode;
use std::collections::HashSet;

/// Multi-bind is a closed receiver that may recursively parse value-producing bodies. Preserve
/// those body's infrastructure failures until the AST emission boundary instead of reducing them
/// to a diagnostic inside this statement-local helper.
type MultiBindResult<T> = Result<T, ExpressionParseError>;

/// One parsed multi-bind statement: a finished node, or a retained continuation.
///
/// WHAT: reports whether the statement completed at parse time or must resume at the body root.
/// WHY: a retained producer may still hold numeric origins, so the value cannot be built yet; its
///      targets resolve from parsed signature slots or real value-block producer routes, and the
///      statement owner records one ordered continuation at the authored position.
// `Completed` is a one-shot statement node pushed straight into the draft, so it stays inline;
// boxing it would add a heap allocation to every parsed multi-bind statement.
#[allow(clippy::large_enum_variant)]
pub(crate) enum MultiBindStatement {
    Completed(AstNode),
    /// The statement waits for its retained producer at the body root.
    Retained(NumericContinuationId),
}

/// One parsed multi-bind right-hand side.
// `Retained` is a one-shot parse result consumed immediately by the statement builder, so it
// stays inline; boxing it would add an allocation to every multi-bind right-hand side.
#[allow(clippy::large_enum_variant)]
enum ParsedMultiBindValue {
    /// A completed expression, including a ready value block.
    Complete(ExprId),
    /// A retained value block, source call, or catch protecting one parsed source call.
    ///
    /// WHY: each shape has an existing owner that supplies its actual result slots and completes
    ///      once at the body root; none becomes a general expression destructuring lane.
    Retained(DeferredMultiBindValue),
}

struct ResolvedMultiBindTargets {
    targets: Vec<DeferredMultiBindTarget>,
    new_declarations: Vec<Declaration>,
}

/// Parsed target syntax retains the name token separately from its annotation anchor so collision
/// validation can wait until the whole target list has passed syntax checking and backtracking.
struct ParsedMultiBindTarget {
    name: StringId,
    name_span: Option<SourceSpan>,
    binding_mode: BindingMode,
    type_annotation: ParsedTypeRef,
    span: Option<SourceSpan>,
}

/// One target slot of a multi-bind right-hand side, known at parse time.
///
/// WHAT: `Known` carries the slot's concrete type, or `None` when only its pending producer routes
///       can resolve it. `Generic` names the declared result shape of a generic call; the slot's
///       type is fixed later by that shape's numeric origins.
/// WHY: a generic result has no exact `TypeId` until completion, so its slot stays a shape link
///      rather than a fabricated type.
#[derive(Clone, Copy)]
enum MultiBindSlot {
    Known(Option<TypeId>),
    Generic(KnownShapeId),
}

/// Per-slot facts of one multi-bind right-hand side, derived at parse time.
///
/// WHAT: the slot of each target, the producer facts that may still resolve it, and
///       the span statement diagnostics report against.
/// WHY: the three facts travel together from slot derivation through arity checking, target
///      resolution and continuation registration, so one named record replaces a tuple.
struct MultiBindSlotFacts {
    rhs_slots: Vec<MultiBindSlot>,
    producer_facts: Vec<Vec<MultiBindProducerFact>>,
    rhs_span: Option<SourceSpan>,
}

// --------------------------
//  Parsing
// --------------------------

pub(crate) fn parse_multi_bind_statement(
    token_stream: &mut AstCursor,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MultiBindResult<Option<MultiBindStatement>> {
    if !has_top_level_comma_before_statement_end(token_stream) {
        return Ok(None);
    }

    let Some(parsed_targets) = parse_target_list(token_stream, string_table)? else {
        return Ok(None);
    };

    validate_unique_target_names(&parsed_targets, string_table)?;
    validate_multi_bind_target_identifiers(&parsed_targets, context, string_table, path_fork)?;
    let first_target = parsed_targets
        .first()
        .expect("a parsed multi-bind must contain at least one target");
    let node_span = first_target.span;
    let target_names = parsed_targets
        .iter()
        .map(|target| target.name)
        .collect::<Vec<_>>();
    let mut rhs_context = context.with_pending_catch_assignment_targets(&target_names);

    let known_slot_types = resolve_known_slot_types(
        &parsed_targets,
        &mut rhs_context,
        type_interner,
        const_values,
        string_table,
        path_fork,
    )?;

    let rhs_value = if token_stream.current_tag() == TokenTag::IF {
        match try_parse_multi_bind_value_block(
            token_stream,
            &rhs_context,
            type_interner,
            expression_builder,
            const_values,
            parsed_targets.len(),
            &known_slot_types,
            string_table,
            path_fork,
        ) {
            Some(Ok(expression)) => ParsedMultiBindValue::Complete(expression.id()),
            Some(Err(error)) => {
                // A numeric-dependent block value owns its deferral here: the whole block is
                // retained once and the body root completes it under the targets' slot
                // expectations, so a nested slot conflict surfaces as the authored assignment
                // mismatch instead of a hard parse error.
                let deferral = error.take_numeric_deferral("a multi-bind value block")?;
                ParsedMultiBindValue::Retained(DeferredMultiBindValue::RetainedExpression {
                    slot_expected_types: known_slot_types.clone(),
                    deferred: Box::new(
                        crate::compiler_frontend::ast::numeric_construction::deferred_numeric_expression(
                            deferral.items,
                            deferral.expected,
                            deferral.value_mode,
                            deferral.span,
                            deferral.suffix,
                        ),
                    ),
                })
            }
            None => parse_multi_bind_rhs_expression(
                token_stream,
                &rhs_context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
            )?,
        }
    } else {
        parse_multi_bind_rhs_expression(
            token_stream,
            &rhs_context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )?
    };

    // Ready source values provide concrete types. A retained value block supplies real slot
    // facts from its reachable `then` values; a slot with no concrete producer remains pending
    // numeric until the body root resolves its actual recipes and receivers.
    let slot_facts = match &rhs_value {
        ParsedMultiBindValue::Complete(expression) => {
            let view = expression_builder.view(*expression)?;
            let slots = extract_rhs_slot_types(view)?;
            let slot_count = slots.len();
            MultiBindSlotFacts {
                rhs_slots: slots
                    .into_iter()
                    .map(|type_id| MultiBindSlot::Known(Some(type_id)))
                    .collect(),
                producer_facts: (0..slot_count).map(|_| Vec::new()).collect(),
                rhs_span: view.span(),
            }
        }
        ParsedMultiBindValue::Retained(value) => {
            retained_multi_bind_slot_facts(value, &rhs_context, expression_builder, const_values)?
        }
    };

    if slot_facts.rhs_slots.len() != parsed_targets.len() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ArityMismatch {
                expected: parsed_targets.len(),
                found: slot_facts.rhs_slots.len(),
            },
            None,
            slot_facts.rhs_span,
        )
        .into());
    }

    let resolved_targets = resolve_multi_bind_targets(
        &parsed_targets,
        &slot_facts,
        &mut *context,
        type_interner,
        const_values,
        string_table,
        path_fork,
    )?;

    register_new_declarations(context, resolved_targets.new_declarations, path_fork);

    let statement = match rhs_value {
        ParsedMultiBindValue::Complete(value) => MultiBindStatement::Completed(AstNode {
            kind: NodeKind::MultiBind {
                targets: require_ready_multi_bind_targets(resolved_targets.targets)?,
                value,
            },
            span: node_span,
            scope: context.scope,
        }),
        // The retained producer routes resume at their selected numeric slot types without
        // changing the value block's own inferred-slot contract.
        ParsedMultiBindValue::Retained(value) => {
            let continuation = context.register_numeric_continuation(
                NumericContinuation::MultiBind(MultiBindContinuation {
                    targets: resolved_targets.targets,
                    producer_routes: retained_producer_routes(&slot_facts.producer_facts),
                    value,
                    span: node_span,
                    scope: context.scope,
                }),
            )?;
            MultiBindStatement::Retained(continuation)
        }
    };

    Ok(Some(statement))
}

/// Check that every target name is legal for assignment and emit naming warnings
/// for newly introduced identifiers.
fn validate_multi_bind_target_identifiers(
    parsed_targets: &[ParsedMultiBindTarget],
    context: &ScopeContext,
    string_table: &mut StringTable,
    _path_fork: &PathInternerFork,
) -> MultiBindResult<()> {
    for target in parsed_targets {
        ensure_not_keyword_shadow_identifier(target.name, target.span, string_table)?;

        if context.is_assignment_target_unavailable(target.name) {
            return Err(CompilerDiagnostic::invalid_assignment_target(
                InvalidAssignmentTargetReason::UnavailableInCatchRecovery,
                Some(target.name),
                None,
                None,
                None,
                None,
                target.span,
            )
            .into());
        }

        if context.get_reference(&target.name).is_none() {
            if let Some(diagnostic) =
                context.check_body_binding_visibility(target.name, target.name_span, string_table)
            {
                return Err(diagnostic.into());
            }
            if let Some(warning) = naming_warning_for_identifier(
                target.name,
                target.span,
                IdentifierNamingKind::ValueLike,
                string_table,
            ) {
                context.emit_warning(warning);
            }
        }
    }

    Ok(())
}

/// Consume a comma-separated list of binding targets and the `=` separator.
///
/// WHY: this is the only place that knows how to backtrack when the stream is not a multi-bind.
fn parse_target_list(
    token_stream: &mut AstCursor,
    string_table: &mut StringTable,
) -> MultiBindResult<Option<Vec<ParsedMultiBindTarget>>> {
    let start_position = token_stream.position();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut parsed_targets = Vec::new();
    let mut saw_comma = false;
    let mut continuation_comma = None;

    loop {
        if token_stream.current_tag() == TokenTag::THIS {
            return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                InvalidMultiBindReason::ThisTargetReserved,
                Some(token_stream.current_span()),
            )
            .into());
        }

        if token_stream.current_tag() != TokenTag::SYMBOL {
            return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                if continuation_comma.is_some() {
                    InvalidMultiBindReason::MissingTargetAfterComma
                } else {
                    InvalidMultiBindReason::ExpectedTargetName
                },
                continuation_comma.unwrap_or_else(|| Some(token_stream.current_span())),
            )
            .into());
        }

        let Some(name) = token_stream.current_string_id_in(string_table)? else {
            return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                if continuation_comma.is_some() {
                    InvalidMultiBindReason::MissingTargetAfterComma
                } else {
                    InvalidMultiBindReason::ExpectedTargetName
                },
                continuation_comma.unwrap_or_else(|| Some(token_stream.current_span())),
            )
            .into());
        };
        let name_span = Some(token_stream.current_span());
        token_stream.advance();
        let target_syntax = {
            let (target_syntax, next_index) = {
                let mut declaration_cursor = token_stream.declaration_cursor()?;
                let target_syntax = parse_binding_target_syntax(
                    name,
                    &mut declaration_cursor,
                    string_table,
                    &mut span_builder,
                )?;
                (target_syntax, declaration_cursor.position())
            };
            token_stream.set_position(next_index)?;
            target_syntax
        };
        let target = ParsedMultiBindTarget {
            name: target_syntax.name,
            name_span,
            binding_mode: target_syntax.binding_mode,
            type_annotation: target_syntax.type_annotation,
            span: target_syntax.span,
        };
        validate_target_mutability(&target, string_table)?;
        parsed_targets.push(target);

        match token_stream.current_tag() {
            TokenTag::COMMA => {
                saw_comma = true;
                let comma_location = Some(token_stream.current_span());
                continuation_comma = Some(comma_location);
                token_stream.advance();

                while token_stream.current_tag() == TokenTag::NEWLINE {
                    token_stream.advance();
                }

                if matches!(
                    token_stream.current_tag(),
                    TokenTag::COMMA | TokenTag::ASSIGN | TokenTag::END | TokenTag::EOF
                ) {
                    return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                        InvalidMultiBindReason::MissingTargetAfterComma,
                        comma_location,
                    )
                    .into());
                }
            }

            TokenTag::ASSIGN => break,

            TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF => {
                return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                    InvalidMultiBindReason::MissingAssignmentOperator,
                    Some(token_stream.current_span()),
                )
                .into());
            }

            _ => {
                return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
                    InvalidMultiBindReason::InvalidTokenAfterTarget,
                    Some(token_stream.current_span()),
                )
                .into());
            }
        }
    }

    if !saw_comma || parsed_targets.len() < 2 {
        token_stream.set_position(start_position)?;
        return Ok(None);
    }

    token_stream.advance();
    Ok(Some(parsed_targets))
}

/// Reject mutable multi-bind targets that lack an explicit type annotation.
fn validate_target_mutability(
    target_syntax: &ParsedMultiBindTarget,
    _string_table: &StringTable,
) -> MultiBindResult<()> {
    if target_syntax.binding_mode.is_mutable()
        && target_syntax.type_annotation.eq(&ParsedTypeRef::Inferred)
    {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::MutableTargetNeedsExplicitType,
            Some(target_syntax.name),
            target_syntax.span,
        )
        .into());
    }

    Ok(())
}

/// Ensure no two targets in the same multi-bind share the same name.
fn validate_unique_target_names(
    parsed_targets: &[ParsedMultiBindTarget],
    _string_table: &StringTable,
) -> MultiBindResult<()> {
    let mut seen_target_names = HashSet::new();
    for target in parsed_targets {
        if !seen_target_names.insert(target.name) {
            return Err(CompilerDiagnostic::invalid_multi_bind(
                InvalidMultiBindReason::DuplicateTarget,
                Some(target.name),
                target.span,
            )
            .into());
        }
    }

    Ok(())
}

/// Parse the single expression on the right-hand side of a multi-bind.
///
/// WHAT: returns a completed value, or exactly one retained source call whose arguments still
///       hold numeric origins.
/// WHY: the right-hand side is this statement's whole syntax, so a deferral raised here is a
///      complete fragment; a fragment that is exactly one retained call has a body-root
///      resumption lane, and every other deferral keeps its unsupported-position classification.
fn parse_multi_bind_rhs_expression(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MultiBindResult<ParsedMultiBindValue> {
    if matches!(
        token_stream.current_tag(),
        TokenTag::NEWLINE | TokenTag::END | TokenTag::EOF
    ) {
        return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
            InvalidMultiBindReason::MissingRightHandExpression,
            Some(token_stream.current_span()),
        )
        .into());
    }
    let mut inferred_rhs_type = ExpectedType::Infer;
    let mut cast_target_context = CastTargetContext::None;
    let rhs_input = ExpressionParseInput::ordinary(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expression_builder,
            const_values,
            expected_type: &mut inferred_rhs_type,
            cast_target_context: &mut cast_target_context,
            value_mode: &ValueMode::ImmutableOwned,
            string_table,
            path_fork,
        },
        false,
    )
    .with_numeric_deferral_owner();
    let rhs_value = match create_expression_with_trailing_newline_policy(rhs_input) {
        Ok(expression) => ParsedMultiBindValue::Complete(expression.id()),
        Err(error) => retain_multi_bind_value(error)?,
    };

    if token_stream.current_tag() == TokenTag::COMMA {
        return Err(CompilerDiagnostic::invalid_multi_bind_syntax(
            InvalidMultiBindReason::MultipleRightHandExpressions,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(rhs_value)
}

/// Accept exactly one retained multi-bind value: a retained source call, or a catch protecting
/// exactly that one parsed source call.
///
/// WHAT: preserves an incomplete body-root numeric deferral, retains the two supported source-call
///       shapes, and reports every complete unsupported RHS through the ordinary multi-bind
///       diagnostic.
/// WHY: only calls and catches have the signature-based resumption lane; a completed variable,
///      field, propagated call, or other expression remains the authored unsupported RHS rather
///      than escaping as an infrastructure failure.
fn retain_multi_bind_value(error: ExpressionParseError) -> MultiBindResult<ParsedMultiBindValue> {
    let ExpressionParseError::NumericDeferral(mut deferral) = error else {
        return Err(error);
    };
    let rhs_span = deferral.span.or_else(|| {
        deferral
            .items
            .first()
            .and_then(ExpressionRpnItem::authored_span)
    });
    if !deferral.complete_fragment {
        return Err(ExpressionParseError::NumericDeferral(deferral));
    }
    if !matches!(deferral.suffix, DeferredExpressionSuffix::None) {
        return Err(unsupported_multi_bind_rhs(rhs_span));
    }
    let Some(item) = into_single_operand(std::mem::take(&mut deferral.items)) else {
        return Err(unsupported_multi_bind_rhs(rhs_span));
    };

    match item {
        ExpressionRpnItem::PendingNumericCall { call, .. } => Ok(ParsedMultiBindValue::Retained(
            DeferredMultiBindValue::Call(*call),
        )),
        ExpressionRpnItem::PendingNumericExpression { expression, .. } => match *expression {
            DeferredExpression::CatchHandler(handler) => {
                let success_slot_count = retained_catch_slots(&handler)?.len();
                Ok(ParsedMultiBindValue::Retained(
                    DeferredMultiBindValue::Catch {
                        handler,
                        success_slot_count,
                    },
                ))
            }
            DeferredExpression::ValueBlock(_) => Err(unsupported_multi_bind_rhs(rhs_span)),
            DeferredExpression::Collection(_) | DeferredExpression::Map(_) => {
                Err(unsupported_multi_bind_rhs(rhs_span))
            }
            DeferredExpression::OperandContinuation(_) => Err(unsupported_multi_bind_rhs(rhs_span)),
        },
        _ => Err(unsupported_multi_bind_rhs(rhs_span)),
    }
}

fn unsupported_multi_bind_rhs(span: Option<SourceSpan>) -> ExpressionParseError {
    CompilerDiagnostic::invalid_multi_bind(InvalidMultiBindReason::UnsupportedRhs, None, span)
        .into()
}

/// The ordered slot types and producer routes of one retained multi-bind value.
///
/// WHAT: calls/catches contribute their parsed signature types; a retained value block contributes
///       known authored receiver types, ready expression types and live retained `then` slots.
/// WHY: unresolved target slots are kept as construction routes, never collapsed into a guessed
///      `TypeId` or rejected merely because their producers have not completed yet.
fn retained_multi_bind_slot_facts(
    value: &DeferredMultiBindValue,
    context: &ScopeContext,
    expression_builder: &AstExpressionBuilder,
    const_values: &ConstValueStore,
) -> MultiBindResult<MultiBindSlotFacts> {
    match value {
        DeferredMultiBindValue::Call(call) => {
            let rhs_slots = retained_bare_call_slots(call)?;
            let producers = (0..rhs_slots.len()).map(|_| Vec::new()).collect();
            Ok(MultiBindSlotFacts {
                rhs_slots,
                producer_facts: producers,
                rhs_span: retained_multi_bind_span(value, expression_builder)?,
            })
        }
        DeferredMultiBindValue::Catch { handler, .. } => {
            let rhs_slots = retained_catch_slots(handler)?;
            let producers = (0..rhs_slots.len()).map(|_| Vec::new()).collect();
            Ok(MultiBindSlotFacts {
                rhs_slots,
                producer_facts: producers,
                rhs_span: retained_multi_bind_span(value, expression_builder)?,
            })
        }
        DeferredMultiBindValue::RetainedExpression {
            slot_expected_types,
            deferred,
        } => {
            let Some(ExpressionRpnItem::PendingNumericExpression { expression, .. }) =
                single_operand(&deferred.items)
            else {
                return Err(CompilerError::compiler_error(
                    "a retained multi-bind value block lost its single deferred value-block item",
                )
                .into());
            };
            let DeferredExpression::ValueBlock(block) = expression.as_ref() else {
                return Err(CompilerError::compiler_error(
                    "a retained multi-bind expression was not owned by the value-block constructor",
                )
                .into());
            };
            let producer_facts = {
                let state = context.numeric_state();
                block.multi_bind_producer_facts(&state, expression_builder, const_values)?
            };
            if slot_expected_types.len() != producer_facts.len() {
                return Err(CompilerError::compiler_error(
                    "a retained multi-bind block's authored slot contract disagreed with its producer list",
                )
                .into());
            }
            let rhs_slots = slot_expected_types
                .iter()
                .copied()
                .zip(producer_facts.iter())
                .map(|(known, producers)| {
                    known.or_else(|| {
                        producers.iter().find_map(|producer| match producer {
                            MultiBindProducerFact::Known(type_id) => Some(*type_id),
                            MultiBindProducerFact::Pending(_) => None,
                        })
                    })
                })
                .map(MultiBindSlot::Known)
                .collect();
            Ok(MultiBindSlotFacts {
                rhs_slots,
                producer_facts,
                rhs_span: deferred.span,
            })
        }
    }
}

/// The authored span of one retained multi-bind value, used by statement-level diagnostics.
fn retained_multi_bind_span(
    value: &DeferredMultiBindValue,
    expression_builder: &AstExpressionBuilder,
) -> MultiBindResult<Option<SourceSpan>> {
    Ok(match value {
        DeferredMultiBindValue::Call(call) => retained_call_span(call),
        DeferredMultiBindValue::Catch { handler, .. } => match &handler.handled {
            StatementExpression::Built(expression) => expression_builder.view(*expression)?.span(),
            StatementExpression::Retained(deferred) => deferred.span,
        },
        // The retained block carries its own authored span.
        DeferredMultiBindValue::RetainedExpression { deferred, .. } => deferred.span,
    })
}

/// Read the ordered success slots of a catch protecting exactly one parsed source call.
///
/// WHAT: prefers the catch's authored site facts when the protected call was already built, and
///       otherwise requires the protected fragment to be exactly one retained terminal call, a
///       bare call or a retained chain's terminal method, with no authored suffix.
/// WHY: a completed catch is a value block, so the parse-time classifier admits it for any call
///      kind whose result has multiple success slots; the retained catch mirrors that (multi-bind
///      always has at least two targets, so the multiple-slot condition is implied), while a handler body
///      or any other protected value stays on the statement-level invariant instead of
///      fabricating an arity.
fn retained_catch_slots(payload: &DeferredCatchHandler) -> MultiBindResult<Vec<MultiBindSlot>> {
    if let Some(site) = &payload.ready_site {
        return Ok(site
            .success_result_type_ids
            .iter()
            .map(|type_id| MultiBindSlot::Known(Some(*type_id)))
            .collect());
    }

    let StatementExpression::Retained(deferred) = &payload.handled else {
        return Err(CompilerError::compiler_error(
            "a retained multi-bind catch carried neither authored site facts nor a retained \
             source call",
        )
        .into());
    };
    if !matches!(deferred.suffix, DeferredExpressionSuffix::None) {
        return Err(CompilerError::compiler_error(
            "a retained multi-bind catch protected a value with an authored propagation suffix",
        )
        .into());
    }
    // The protected fragment is exactly one retained terminal call: a source, concrete-receiver,
    // or external call, or the terminal unhandled method of a retained chain. Its ordered slots
    // come from the parsed signature, never from a general expression.
    let terminal = single_operand(&deferred.items).and_then(retained_terminal_call);
    let Some(terminal) = terminal else {
        // A retained catch around anything but one call (an authored arithmetic or otherwise
        // single-result fragment) is ordinary multi-bind source misuse, not corrupted construction
        // state: report the same structured RHS shape diagnostic the completed classifier reports,
        // at the authored span, instead of a construction invariant failure.
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::RhsNotMultiValue,
            None,
            deferred.span,
        )
        .into());
    };
    // A generic protected call's slots are its declared result shapes, typed by its origins.
    retained_call_slots(terminal)
}

/// The ordered success slots of a retained call in the bare multi-bind position.
///
/// WHAT: reads the retained call's slots for the call kinds the completed classifier accepts:
///       free source calls, generic source calls and host calls. A receiver method call, struct
///       constructor, choice constructor or generic nominal constructor reports the completed
///       path's own `UnsupportedRhs`.
/// WHY: the retained path must accept exactly what the parse-time path accepts; a bare method call
///      is not a multi-bind value (only free source calls, host calls and value blocks are), so
///      its retained form must not become legal by accident of retention.
fn retained_bare_call_slots(call: &DeferredCall) -> MultiBindResult<Vec<MultiBindSlot>> {
    if matches!(
        call,
        DeferredCall::Receiver(_)
            | DeferredCall::Struct(_)
            | DeferredCall::Choice(_)
            | DeferredCall::GenericNominal(_)
    ) {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::UnsupportedRhs,
            None,
            call.call_span(),
        )
        .into());
    }
    retained_call_slots(RetainedTerminalCall::Call(call))
}

/// The ordered success slots of a retained call, as multi-bind resolution consumes them.
///
/// WHAT: a generic call contributes one `Generic` slot per declared success shape; every other
///       call contributes its parsed success types as `Known` slots.
/// WHY: a generic slot's type is fixed only by the call's numeric origins at completion, so its
///      target links to the declared shape instead of a guessed `TypeId`.
fn retained_call_slots(call: RetainedTerminalCall<'_>) -> MultiBindResult<Vec<MultiBindSlot>> {
    if let Some(shapes) = call.generic_result_shapes() {
        return Ok(shapes
            .iter()
            .map(|shape| MultiBindSlot::Generic(*shape))
            .collect());
    }
    let Some(slots) = call.success_return_type_ids() else {
        return Err(CompilerError::compiler_error(
            "a non-generic retained call reported no parsed success slot types",
        )
        .into());
    };
    Ok(slots
        .into_iter()
        .map(|type_id| MultiBindSlot::Known(Some(type_id)))
        .collect())
}

/// The authored span of a retained call, used by statement-level diagnostics.
fn retained_call_span(call: &DeferredCall) -> Option<SourceSpan> {
    call.call_span()
}

fn resolve_known_slot_types(
    parsed_targets: &[ParsedMultiBindTarget],
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    _path_fork: &PathInternerFork,
) -> MultiBindResult<Vec<Option<TypeId>>> {
    let mut known = Vec::with_capacity(parsed_targets.len());

    for target_syntax in parsed_targets {
        let explicit_type = resolve_target_explicit_type(
            target_syntax,
            context,
            type_interner,
            const_values,
            string_table,
        )?;
        if let Some((type_id, _)) = explicit_type {
            known.push(Some(type_id));
            continue;
        }

        if let Some(existing_reference) = context.get_reference(&target_syntax.name)
            && let Some(existing) = existing_reference.ready_declaration()
        {
            known.push(Some(existing.type_id));
            continue;
        }

        known.push(None);
    }

    Ok(known)
}

/// Register the known receiver of every generic result slot before any target is created.
///
/// WHAT: an explicitly typed target, or the ready declaration it names, is the receiver of its
///       slot's generic shape. Its exact leaves join that shape's origin class.
/// WHY: every result envelope of one generic origin must see the same receivers. Joining them all
///      first means an unannotated sibling resolves the same way whichever order the targets are
///      written in.
fn register_generic_slot_receivers(
    slot_facts: &MultiBindSlotFacts,
    parsed_targets: &[ParsedMultiBindTarget],
    explicit_types: &[Option<(TypeId, DataType)>],
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
) {
    for ((slot, target_syntax), explicit_type) in slot_facts
        .rhs_slots
        .iter()
        .zip(parsed_targets)
        .zip(explicit_types)
    {
        let MultiBindSlot::Generic(shape) = *slot else {
            continue;
        };
        let receiver = match explicit_type {
            Some((type_id, _)) => Some(*type_id),
            None => context
                .get_reference(&target_syntax.name)
                .and_then(|reference| {
                    reference
                        .ready_declaration()
                        .map(|declaration| declaration.type_id)
                }),
        };
        let Some(receiver) = receiver else {
            continue;
        };
        // Reference lookup reads numeric binding metadata, so finish it before the mutable borrow.
        let mut state = context.numeric_state_mut();
        register_shape_witness(
            &mut state,
            type_interner.environment(),
            shape,
            receiver,
            target_syntax.span,
        );
    }
}

/// The type of an unannotated generic result slot once its shape projects to an exact type.
///
/// WHAT: a slot with its own known type keeps it. Otherwise a generic slot takes the exact type its
///       shape projects to, and an open shape answers `None`.
/// WHY: results from one generic origin share its exact width and precision. An unsolved leaf is
///      never defaulted here, so the slot stays pending for its own completion.
fn resolve_generic_slot_type(
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    slot_type: Option<TypeId>,
    generic_shape: Option<KnownShapeId>,
) -> MultiBindResult<Option<TypeId>> {
    if slot_type.is_some() {
        return Ok(slot_type);
    }
    let Some(shape) = generic_shape else {
        return Ok(None);
    };
    let mut state = context.numeric_state_mut();
    exact_shape_type(
        &mut state,
        type_interner.environment_mut_for_derived_types(),
        shape,
    )
}

/// Verify that the RHS expression belongs to a whitelisted multi-value-producing surface.
fn classify_multi_bind_rhs(expression: ExpressionView<'_>) -> MultiBindResult<()> {
    match expression.operation() {
        ExpressionOperationView::Call(_) => Ok(()),

        ExpressionOperationView::ValueBlock(_) if expression.success_types().len() >= 2 => Ok(()),
        ExpressionOperationView::ValueBlock(_) => Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::RhsNotMultiValue,
            None,
            expression.span(),
        )
        .into()),

        _ => Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::UnsupportedRhs,
            None,
            expression.span(),
        )
        .into()),
    }
}

/// Extract the ordered slot `TypeId`s from a classified multi-value RHS expression.
fn extract_rhs_slot_types(rhs_expression: ExpressionView<'_>) -> MultiBindResult<Vec<TypeId>> {
    classify_multi_bind_rhs(rhs_expression)?;

    let slots = rhs_expression.success_types();
    if slots.len() < 2 {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::RhsNotMultiValue,
            None,
            rhs_expression.span(),
        )
        .into());
    }

    Ok(slots.to_vec())
}

/// Resolve each authored target from a known producer type or a pending producer route.
fn resolve_multi_bind_targets(
    parsed_targets: &[ParsedMultiBindTarget],
    slot_facts: &MultiBindSlotFacts,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> MultiBindResult<ResolvedMultiBindTargets> {
    let mut resolved_bindings = Vec::with_capacity(parsed_targets.len());
    let mut new_declarations = Vec::new();

    let mut explicit_types = Vec::with_capacity(parsed_targets.len());
    for target_syntax in parsed_targets {
        explicit_types.push(resolve_target_explicit_type(
            target_syntax,
            context,
            type_interner,
            const_values,
            string_table,
        )?);
    }
    register_generic_slot_receivers(
        slot_facts,
        parsed_targets,
        &explicit_types,
        context,
        type_interner,
    );

    for (slot_index, ((slot, target_syntax), explicit_type)) in slot_facts
        .rhs_slots
        .iter()
        .zip(parsed_targets.iter())
        .zip(explicit_types)
        .enumerate()
    {
        let producers = slot_facts.producer_facts.get(slot_index).ok_or_else(|| {
            CompilerError::compiler_error(
                "multi-bind target resolution lost its corresponding producer slot",
            )
        })?;
        let target_ownership = binding_target_ownership(target_syntax);
        let explicit_type_id = explicit_type.as_ref().map(|(type_id, _)| *type_id);
        let explicit_diagnostic_type = explicit_type.map(|(_, diagnostic_type)| diagnostic_type);
        let (rhs_type, generic_shape) = match *slot {
            MultiBindSlot::Known(type_id) => (type_id, None),
            MultiBindSlot::Generic(shape) => (None, Some(shape)),
        };
        let slot_type = rhs_type.or(explicit_type_id);

        let Some(existing_reference) = context.get_reference(&target_syntax.name) else {
            let target_id = path_fork
                .try_intern_child(context.scope, target_syntax.name)
                .expect("path table exhausted while creating multi-bind target");
            let target_span = target_syntax.span;

            // An unannotated generic slot takes the exact type of its shape once every receiver is
            // registered, so results from one origin keep the exact width and precision.
            let slot_type =
                resolve_generic_slot_type(context, type_interner, slot_type, generic_shape)?;
            let Some(slot_type) = slot_type else {
                if let Some(element) = generic_shape {
                    // The slot's type resolves from its declared shape at completion.
                    let binding = context.declare_pending_numeric_binding(
                        AuthoredBindingFacts {
                            name: target_syntax.name,
                            path: target_id,
                            span: target_span,
                            binding_span: target_span,
                            value_mode: target_ownership,
                            mutable: false,
                        },
                        NumericRecipe::CollectionElement { element },
                    )?;
                    // Solved leaves stay exact in the read view while unsolved leaves stay pending.
                    {
                        let mut state = context.numeric_state_mut();
                        let view = project_exact_shape(&mut state, element)?;
                        register_pending_reference_view(&mut state, binding, view);
                    }
                    resolved_bindings.push(DeferredMultiBindTarget::Pending {
                        binding,
                        kind: MultiBindTargetKind::Declaration,
                        span: target_span,
                        producer_resolves_type: true,
                    });
                    continue;
                }
                let producers = pending_multi_bind_producers(producers);
                if producers.is_empty() {
                    return Err(CompilerDiagnostic::invalid_control_flow_statement(
                        InvalidControlFlowStatementReason::ValueIfNoProducingPath,
                        slot_facts.rhs_span,
                    )
                    .into());
                }
                let shape = {
                    let mut state = context.numeric_state_mut();
                    merged_multi_bind_shape(&mut state, type_interner.environment(), &producers)
                };
                let binding = context.declare_pending_numeric_binding(
                    AuthoredBindingFacts {
                        name: target_syntax.name,
                        path: target_id,
                        span: target_span,
                        binding_span: target_span,
                        value_mode: target_ownership,
                        mutable: false,
                    },
                    NumericRecipe::MultiBindSlot { producers, shape },
                )?;
                resolved_bindings.push(DeferredMultiBindTarget::Pending {
                    binding,
                    kind: MultiBindTargetKind::Declaration,
                    span: target_span,
                    producer_resolves_type: true,
                });
                continue;
            };

            let target_data_type = resolve_new_target_data_type(
                target_syntax,
                explicit_type_id,
                explicit_diagnostic_type.as_ref(),
                slot_type,
                slot_index,
                string_table,
                type_interner.environment(),
            )?;
            new_declarations.push(Declaration {
                id: target_id.to_owned(),
                type_id: slot_type,
                diagnostic_type: target_data_type.to_owned(),
                value_mode: target_ownership.to_owned(),
                value: DeclarationValue::Absent,
                binding_span: target_span,
                config_qualifier: None,
            });
            register_direct_producer_witnesses(
                context,
                producers,
                slot_type,
                type_interner.environment(),
            );
            resolved_bindings.push(DeferredMultiBindTarget::Ready(MultiBindTarget {
                id: target_id,
                type_id: slot_type,
                value_mode: target_ownership,
                kind: MultiBindTargetKind::Declaration,
                span: target_span,
            }));
            continue;
        };

        if let Some(declaration) = existing_reference.ready_declaration() {
            let slot_type = match generic_shape {
                // A generic slot without its own type takes the declaration's type.
                Some(_) => slot_type.unwrap_or(declaration.type_id),
                None => slot_type.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "a ready multi-bind target had no known producer type",
                    )
                })?,
            };
            let target = resolve_existing_target(
                target_syntax,
                slot_type,
                explicit_type_id.as_ref(),
                declaration,
                slot_index,
                string_table,
                type_interner.environment(),
            )?;
            register_direct_producer_witnesses(
                context,
                producers,
                slot_type,
                type_interner.environment(),
            );
            resolved_bindings.push(DeferredMultiBindTarget::Ready(target));
            continue;
        }

        let binding = existing_reference
            .pending_binding()
            .expect("a non-ready reference is a pending numeric binding");
        let Some(mutable) = context.pending_numeric_is_mutable(binding) else {
            return Err(CompilerError::compiler_error(
                "multi-bind target has a stale pending numeric binding",
            )
            .into());
        };
        if target_syntax.binding_mode.is_mutable() {
            return Err(CompilerDiagnostic::invalid_multi_bind(
                InvalidMultiBindReason::ExistingTargetMutableMarker,
                Some(target_syntax.name),
                target_syntax.span,
            )
            .into());
        }
        if !mutable {
            return Err(CompilerDiagnostic::invalid_multi_bind(
                InvalidMultiBindReason::ExistingTargetImmutable,
                Some(target_syntax.name),
                target_syntax.span,
            )
            .into());
        }
        if let (Some(explicit_type_id), Some(slot_type)) = (explicit_type_id, slot_type)
            && explicit_type_id != slot_type
        {
            return Err(CompilerDiagnostic::type_mismatch(
                explicit_type_id,
                slot_type,
                TypeMismatchContext::General,
                target_syntax.span,
            )
            .into());
        }
        let (path, value_mode) = {
            let state = context.numeric_state();
            let row = state.binding(binding);
            (row.path, row.value_mode.to_owned())
        };
        {
            let mut state = context.numeric_state_mut();
            if let Some(type_id) = slot_type {
                register_binding_witness(
                    &mut state,
                    type_interner.environment(),
                    binding,
                    type_id,
                    target_syntax.span,
                );
            } else if let Some(shape) = generic_shape {
                // An established result type is an incoming value, not another raw origin that
                // can import its pre-binding receivers into the target's already bound family.
                let shape = project_exact_shape(&mut state, shape)?;
                union_pending_binding_with_generic_slot(
                    &mut state,
                    type_interner.environment(),
                    binding,
                    shape,
                );
            }
            union_direct_producer_classes(&mut state, binding, producers);
        }

        if let Some(slot_type) = slot_type {
            resolved_bindings.push(DeferredMultiBindTarget::Ready(MultiBindTarget {
                id: path,
                type_id: slot_type,
                value_mode,
                kind: MultiBindTargetKind::Assignment,
                span: target_syntax.span,
            }));
        } else {
            resolved_bindings.push(DeferredMultiBindTarget::Pending {
                binding,
                kind: MultiBindTargetKind::Assignment,
                span: target_syntax.span,
                producer_resolves_type: false,
            });
        }
    }

    Ok(ResolvedMultiBindTargets {
        targets: resolved_bindings,
        new_declarations,
    })
}

fn pending_multi_bind_producers(facts: &[MultiBindProducerFact]) -> Vec<PendingMultiBindProducer> {
    facts
        .iter()
        .filter_map(|fact| match fact {
            MultiBindProducerFact::Known(_) => None,
            MultiBindProducerFact::Pending(producer) => Some(*producer),
        })
        .collect()
}

/// The one known shape a multi-bind slot's producers share, merged once at declaration.
///
/// WHAT: a binding source contributes the shape it resolves to; a literal producer contributes the
///       shape its own fragment owns. The first known shape is the carrier. Each later one joins it
///       unless the pair is refused, in which case it stays separate.
/// WHY: the slot owns one merged shape, so receivers and loop headers read it without re-querying
///      every source. Scalar producers have no shape and never invent one.
fn merged_multi_bind_shape(
    state: &mut NumericConstructionState,
    environment: &TypeEnvironment,
    producers: &[PendingMultiBindProducer],
) -> Option<KnownShapeId> {
    let mut carrier = None;
    for producer in producers {
        let incoming = match producer.kind {
            PendingMultiBindProducerKind::Binding(source) => binding_reference_shape(state, source),
            PendingMultiBindProducerKind::Raw(_) | PendingMultiBindProducerKind::Forward => {
                producer.shape
            }
        };
        let Some(incoming) = incoming else {
            continue;
        };
        match carrier {
            None => carrier = Some(incoming),
            Some(existing) => {
                if !shapes_join_refused(state, environment, existing, incoming) {
                    union_shape_classes(state, environment, existing, incoming);
                }
            }
        }
    }
    carrier
}

fn retained_producer_routes(
    producer_facts: &[Vec<MultiBindProducerFact>],
) -> Vec<(usize, PendingMultiBindProducer)> {
    let route_count = producer_facts
        .iter()
        .flatten()
        .filter(|fact| {
            matches!(
                fact,
                MultiBindProducerFact::Pending(PendingMultiBindProducer {
                    kind: PendingMultiBindProducerKind::Raw(_)
                        | PendingMultiBindProducerKind::Forward,
                    ..
                })
            )
        })
        .count();
    let mut routes = Vec::with_capacity(route_count);
    for (slot_index, facts) in producer_facts.iter().enumerate() {
        for fact in facts {
            if let MultiBindProducerFact::Pending(producer) = fact
                && matches!(
                    producer.kind,
                    PendingMultiBindProducerKind::Raw(_) | PendingMultiBindProducerKind::Forward
                )
            {
                routes.push((slot_index, *producer));
            }
        }
    }
    routes
}

fn register_direct_producer_witnesses(
    context: &ScopeContext,
    facts: &[MultiBindProducerFact],
    type_id: TypeId,
    environment: &TypeEnvironment,
) {
    let mut state = context.numeric_state_mut();
    for fact in facts {
        if let MultiBindProducerFact::Pending(PendingMultiBindProducer {
            kind: PendingMultiBindProducerKind::Binding(binding),
            span,
            ..
        }) = fact
        {
            register_binding_witness(&mut state, environment, *binding, type_id, *span);
        }
    }
}

fn union_direct_producer_classes(
    state: &mut crate::compiler_frontend::ast::numeric_construction::NumericConstructionState,
    target: crate::compiler_frontend::ast::numeric_construction::NumericBindingId,
    facts: &[MultiBindProducerFact],
) {
    for fact in facts {
        if let MultiBindProducerFact::Pending(PendingMultiBindProducer {
            kind: PendingMultiBindProducerKind::Binding(source),
            ..
        }) = fact
        {
            union_alias_classes(state, target, *source);
        }
    }
}

fn require_ready_multi_bind_targets(
    targets: Vec<DeferredMultiBindTarget>,
) -> MultiBindResult<Vec<MultiBindTarget>> {
    targets
        .into_iter()
        .map(|target| match target {
            DeferredMultiBindTarget::Ready(target) => Ok(target),
            DeferredMultiBindTarget::Pending { .. } => Err(CompilerError::compiler_error(
                "a completed multi-bind value retained a pending target route",
            )
            .into()),
        })
        .collect()
}

// --------------------------
//  Target resolution
// --------------------------

/// Map the parsed binding mode to the value mode used for the target declaration.
fn binding_target_ownership(target_syntax: &ParsedMultiBindTarget) -> ValueMode {
    target_syntax.binding_mode.value_mode()
}

/// Validate that an existing mutable local is compatible with the corresponding RHS slot.
fn resolve_existing_target(
    target_syntax: &ParsedMultiBindTarget,
    slot_type: TypeId,
    explicit_type_id: Option<&TypeId>,
    existing_declaration: &Declaration,
    _slot_index: usize,
    _string_table: &StringTable,
    _type_environment: &TypeEnvironment,
) -> MultiBindResult<MultiBindTarget> {
    if target_syntax.binding_mode.is_mutable() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ExistingTargetMutableMarker,
            Some(target_syntax.name),
            target_syntax.span,
        )
        .into());
    }

    if !existing_declaration.value_mode.is_mutable() {
        return Err(CompilerDiagnostic::invalid_multi_bind(
            InvalidMultiBindReason::ExistingTargetImmutable,
            Some(target_syntax.name),
            target_syntax.span,
        )
        .into());
    }

    if let Some(explicit_type_id) = explicit_type_id
        && *explicit_type_id != existing_declaration.type_id
    {
        return Err(CompilerDiagnostic::type_mismatch(
            existing_declaration.type_id,
            *explicit_type_id,
            TypeMismatchContext::General,
            target_syntax.span,
        )
        .into());
    }

    if existing_declaration.type_id != slot_type {
        return Err(CompilerDiagnostic::type_mismatch(
            existing_declaration.type_id,
            slot_type,
            TypeMismatchContext::General,
            target_syntax.span,
        )
        .into());
    }

    Ok(MultiBindTarget {
        id: existing_declaration.id.to_owned(),
        type_id: slot_type,
        value_mode: existing_declaration.value_mode.to_owned(),
        kind: MultiBindTargetKind::Assignment,
        span: target_syntax.span,
    })
}

/// Determine the diagnostic `DataType` for a new declaration target.
///
/// If the target has an explicit annotation, it is checked against the corresponding RHS slot type.
fn resolve_new_target_data_type(
    target_syntax: &ParsedMultiBindTarget,
    explicit_type_id: Option<TypeId>,
    explicit_diagnostic_type: Option<&DataType>,
    slot_type: TypeId,
    _slot_index: usize,
    _string_table: &StringTable,
    type_environment: &TypeEnvironment,
) -> MultiBindResult<DataType> {
    let Some(explicit_type_id) = explicit_type_id else {
        // Display-only diagnostic spelling for the declaration.
        return Ok(diagnostic_type_spelling(slot_type, type_environment));
    };

    if explicit_type_id != slot_type {
        return Err(CompilerDiagnostic::type_mismatch(
            explicit_type_id,
            slot_type,
            TypeMismatchContext::General,
            target_syntax.span,
        )
        .into());
    }

    // Return the explicit annotation's diagnostic spelling if available.
    // Inferred targets derive display spelling from the canonical TypeId.
    Ok(explicit_diagnostic_type
        .cloned()
        .unwrap_or_else(|| diagnostic_type_spelling(slot_type, type_environment)))
}

fn register_new_declarations(
    context: &mut ScopeContext,
    new_declarations: Vec<Declaration>,
    path_fork: &PathInternerFork,
) {
    for declaration in new_declarations {
        let binding_span = declaration.binding_span;
        context.add_var(declaration, binding_span, path_fork);
    }
}

// --------------------------
//  Helpers
fn resolve_target_explicit_type(
    target_syntax: &ParsedMultiBindTarget,
    context: &mut ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
) -> MultiBindResult<Option<(TypeId, DataType)>> {
    if target_syntax.type_annotation.eq(&ParsedTypeRef::Inferred) {
        return Ok(None);
    }

    let resolved_annotation = {
        let mut type_resolution_context =
            TypeResolutionContext::from_inputs(TypeResolutionContextInputs {
                declaration_table: &context.top_level_declarations,
                visible_declaration_ids: context.visible_declaration_ids.as_ref(),
                visible_external_symbols: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_external_symbols),
                visible_source_bindings: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_source_names),
                visible_type_aliases: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_type_alias_names),
                resolved_type_aliases: context.resolved_type_aliases.as_deref(),
                generic_declarations_by_path: context.generic_declarations_by_path.as_deref(),
                resolved_struct_fields_by_path: context.resolved_struct_fields_by_path.as_deref(),
                type_environment: type_interner.environment_mut_for_derived_types(),
                const_values,
                visible_namespace_records: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_namespace_records),
                trait_environment: Some(context.trait_environment()),
                trait_evidence_environment: Some(context.trait_evidence_environment()),
                visible_trait_names: context
                    .file_visibility
                    .as_ref()
                    .map(|fv| &fv.visible_trait_names),
            })
            .with_active_generic_type_context(context.active_generic_type_context());

        resolve_parsed_type_annotation(
            target_syntax.type_annotation.clone(),
            target_syntax.span,
            &mut type_resolution_context,
            string_table,
            Some(context),
        )?
    };

    // Only an inferred placeholder has no canonical identity yet, so the target stays
    // unannotated. Every other resolved annotation already carries its identity, and an
    // unresolved nominal fails during checked resolution above.
    let Some(type_id) = resolved_annotation.type_id else {
        return Ok(None);
    };

    Ok(Some((type_id, resolved_annotation.diagnostic_type)))
}
