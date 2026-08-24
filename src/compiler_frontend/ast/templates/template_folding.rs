//! Compile-time template folding.
//!
//! WHAT: Converts finalized TIR-backed template trees and const control-flow
//! bodies into interned string IDs.
//!
//! WHY: Keeps compile-time folding inside AST template preparation and shares
//! the same finalized template semantics that later runtime handoff consumes,
//! without entangling parser or HIR code.

use crate::compiler_frontend::ast::const_eval::constant_fold;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpn, ExpressionRpnItem,
};
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateFoldBinding, TemplateLoopControlKind,
};
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, TemplateIrStore, TemplatePreparationMode, TemplateTirPhase, TirView,
    prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

// -------------------------
//  Folding Context
// -------------------------

/// Narrow state shared by TIR constant folding and its AST expression helpers.
///
/// The exact TIR view owns structural authority. Project path and formatting services stay at the
/// outer AST boundaries that consume them rather than travelling through the TIR reducer.
pub(crate) struct TirFoldContext<'a> {
    pub string_table: &'a mut StringTable,
    pub template_const_loop_iteration_limit: usize,

    pub(crate) bindings: Vec<TemplateFoldBinding>,
}

/// Compile-time template folding must keep structural no-output distinct from
/// output that happens to be an empty string, because parent wrappers apply only
/// to structurally emitted children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TemplateEmission {
    NoOutput,
    Output(StringId),
    Break(Option<StringId>),
    Continue(Option<StringId>),
}

/// Exact fold output paired with the semantic provenance consumed to produce it.
///
/// WHAT: keeps text emission and the canonical synthetic-interface dependency set together for
///      every exact TIR fold, including recursive child and wrapper folds.
/// WHY: provenance must follow the selected fold path and must stay attached to the result it was
///      consumed for; an ambient accumulator would leak unselected branches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TemplateFoldResult {
    pub(crate) emission: TemplateEmission,
    pub(crate) provenance: SyntheticInterfaceProvenance,
    pub(crate) projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
}

impl TemplateFoldResult {
    pub(crate) fn with_projection(
        emission: TemplateEmission,
        provenance: SyntheticInterfaceProvenance,
        projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
    ) -> Self {
        Self {
            emission,
            provenance,
            projection_pieces,
        }
    }
}

/// Borrow-first expression resolution result for template folding.
///
/// WHAT: distinguishes expressions that were not modified during fold-binding
///       resolution (borrowed reference to the original) from expressions that
///       were actually rewritten (owned).
/// WHY: most template expressions pass through folding unchanged because they
///      contain no foldable bindings. Returning a borrowed reference avoids
///      cloning the entire expression tree on the common no-substitution path,
///      which is the majority of expressions in template-heavy modules.
pub(crate) enum FoldResolvedExpression<'a> {
    /// The expression was not changed; fold sites can use the original.
    Borrowed(&'a Expression),
    /// The expression was actually rewritten; this is the owned result.
    Owned(Box<Expression>),
}

impl FoldResolvedExpression<'_> {
    /// Consumes the resolved expression and returns an owned `Expression`.
    ///
    /// WHAT: clones only when the resolved expression is borrowed (no substitution
    ///       happened), so callers that genuinely need an owned value still work.
    /// WHY: a few call sites (like RPN operand vectors) need owned values, but
    ///      this method makes the clone explicit and only happens when the
    ///      borrow-first path determined a rewrite is required.
    pub(crate) fn into_owned(self) -> Expression {
        match self {
            FoldResolvedExpression::Borrowed(expr) => expr.clone(),
            FoldResolvedExpression::Owned(expr) => *expr,
        }
    }
}

impl TirFoldContext<'_> {
    fn lookup_binding(&self, path: &InternedPath) -> Option<&Expression> {
        self.bindings
            .iter()
            .rev()
            .find(|binding| &binding.path == path)
            .map(|binding| &binding.value)
    }

    pub(crate) fn push_bindings(
        &mut self,
        bindings: impl IntoIterator<Item = TemplateFoldBinding>,
    ) -> usize {
        let previous_len = self.bindings.len();
        self.bindings.extend(bindings);
        previous_len
    }

    pub(crate) fn restore_bindings(&mut self, previous_len: usize) {
        self.bindings.truncate(previous_len);
    }
}

// -------------------------
//  Folding Implementation
// -------------------------

/// Resolves one option-capture scrutinee and returns both its binding and the provenance consumed
/// while deciding whether the capture is present.
pub(crate) fn selected_option_capture_payload_with_provenance(
    scrutinee: &Expression,
    pattern: &MatchPattern,
    store: &TemplateIrStore,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<(Option<TemplateFoldBinding>, SyntheticInterfaceProvenance), TemplateError> {
    match const_option_presence(scrutinee, store, fold_context)? {
        ConstOptionPresence::Present { value, provenance } => Ok((
            Some(TemplateFoldBinding {
                path: option_capture_binding_path(pattern)?,
                value: *value,
            }),
            provenance,
        )),

        ConstOptionPresence::Absent { provenance } => Ok((None, provenance)),
    }
}

enum ConstOptionPresence {
    Present {
        value: Box<Expression>,
        provenance: SyntheticInterfaceProvenance,
    },
    Absent {
        provenance: SyntheticInterfaceProvenance,
    },
}

fn const_option_presence(
    scrutinee: &Expression,
    store: &TemplateIrStore,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<ConstOptionPresence, TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(scrutinee, fold_context)?;

    // Work with the resolved expression by reference to avoid an extra clone
    // when the resolver returned a borrowed reference (no binding was substituted).
    let resolved_ref: &Expression = match &resolved {
        FoldResolvedExpression::Borrowed(expr) => expr,
        FoldResolvedExpression::Owned(expr) => expr,
    };

    match &resolved_ref.kind {
        ExpressionKind::OptionNone => Ok(ConstOptionPresence::Absent {
            provenance: resolved_ref.synthetic_interface_provenance.clone(),
        }),

        ExpressionKind::Coerced { value, .. } => {
            let payload = (**value).clone();

            // Scalar and other non-template payloads keep their ordinary const rules.
            // The active fold view supplies the store authority for any nested
            // template reached by expression recursion.
            let payload_is_compile_time_constant = payload
                .const_value_kind_with_template_classifier(&mut |template| {
                    let reference = template.tir_reference;
                    let view = TirView::with_minimum_phase(
                        store,
                        reference.root,
                        reference.phase,
                        TemplateTirPhase::Composed,
                        reference.context,
                    )?;
                    Ok(prepare_tir_view(&view, TemplatePreparationMode::Value)?
                        .facts
                        .final_value_kind)
                })?
                .is_compile_time_value();

            if payload_is_compile_time_constant {
                Ok(ConstOptionPresence::Present {
                    provenance: resolved_ref
                        .synthetic_interface_provenance
                        .union(&payload.synthetic_interface_provenance),
                    value: Box::new(payload),
                })
            } else {
                Err(option_capture_const_deferred_error(resolved_ref).into())
            }
        }

        _ => Err(option_capture_const_deferred_error(resolved_ref).into()),
    }
}

fn option_capture_binding_path(pattern: &MatchPattern) -> Result<InternedPath, TemplateError> {
    let MatchPattern::OptionPresentCapture { binding_path, .. } = pattern else {
        return Err(CompilerError::compiler_error(
            "Template option-capture folding received a non-capture pattern.",
        )
        .into());
    };

    Ok(binding_path.clone())
}

fn option_capture_const_deferred_error(expression: &Expression) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_template_structure(
        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
        expression.location.clone(),
    )
}

pub(crate) fn fold_conditional_loop_const_condition(
    condition: &Expression,
    location: &SourceLocation,
) -> Result<bool, TemplateError> {
    match &condition.kind {
        ExpressionKind::Bool(value) => Ok(*value),

        ExpressionKind::Coerced { value, .. } => {
            fold_conditional_loop_const_condition(value, location)
        }

        _ => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateLoopConditionNotConst,
            condition_location_or_loop_location(condition, location),
        )
        .into()),
    }
}

pub(crate) fn condition_location_or_loop_location(
    condition: &Expression,
    loop_location: &SourceLocation,
) -> SourceLocation {
    if condition.location == Default::default() {
        loop_location.clone()
    } else {
        condition.location.clone()
    }
}

/// Evaluates a const template condition and returns the exact value provenance consumed by it.
pub(crate) fn fold_bool_condition_with_provenance(
    condition: &Expression,
    fallback_location: &SourceLocation,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<(bool, SyntheticInterfaceProvenance), TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(condition, fold_context)?;

    // Borrow the resolved expression by reference to avoid cloning when no
    // binding was substituted (the common path for const template conditions).
    let resolved_ref: &Expression = match &resolved {
        FoldResolvedExpression::Borrowed(expr) => expr,
        FoldResolvedExpression::Owned(expr) => expr,
    };

    let value = fold_resolved_bool_condition(resolved_ref, fallback_location)?;
    Ok((value, resolved_ref.synthetic_interface_provenance.clone()))
}

fn fold_resolved_bool_condition(
    condition: &Expression,
    fallback_location: &SourceLocation,
) -> Result<bool, TemplateError> {
    match &condition.kind {
        ExpressionKind::Bool(value) => Ok(*value),
        ExpressionKind::Coerced { value, .. } => {
            fold_resolved_bool_condition(value, fallback_location)
        }
        _ => Err(CompilerDiagnostic::invalid_template_structure(
            InvalidTemplateStructureReason::TemplateIfConditionNotConst,
            if condition.location == Default::default() {
                fallback_location.clone()
            } else {
                condition.location.clone()
            },
        )
        .into()),
    }
}

pub(crate) fn template_emission_from_output_and_signal(
    output: StringId,
    signal_kind: Option<TemplateLoopControlKind>,
) -> TemplateEmission {
    match signal_kind {
        None => TemplateEmission::Output(output),
        Some(TemplateLoopControlKind::Break) => TemplateEmission::Break(Some(output)),
        Some(TemplateLoopControlKind::Continue) => TemplateEmission::Continue(Some(output)),
    }
}

/// Resolves fold bindings in an expression using a borrow-first strategy.
///
/// WHAT: examines an expression and returns either a borrowed reference to the
///       original (when no substitution was needed) or an owned rewritten expression.
/// WHY: most template expressions contain no foldable bindings. Cloning the
///      entire expression tree on every fold call is wasted work when the common
///      path simply passes the expression through unchanged. The borrow-first
///      approach avoids allocation on the no-substitution path entirely.
pub(crate) fn resolve_fold_bindings_in_expression<'a>(
    expression: &'a Expression,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<FoldResolvedExpression<'a>, TemplateError> {
    match &expression.kind {
        ExpressionKind::Reference(path) => {
            if let Some(bound_value) = fold_context.lookup_binding(path) {
                // Binding found: produce an owned clone of the bound value.
                // This is the actual substitution that justifies an allocation.
                add_ast_counter(AstCounter::TemplateFoldBindingSubstitutions, 1);
                add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
                add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 1);
                Ok(FoldResolvedExpression::Owned(Box::new(bound_value.clone())))
            } else {
                // No binding: borrow the original expression unchanged.
                Ok(FoldResolvedExpression::Borrowed(expression))
            }
        }

        ExpressionKind::Coerced { value, to_type } => {
            let resolved = resolve_fold_bindings_in_expression(value, fold_context)?;

            // If the inner value was not substituted, the coerced wrapper is
            // also unchanged — borrow the original expression.
            if matches!(resolved, FoldResolvedExpression::Borrowed(_)) {
                // A coercion wrapper around a template expression is transparent
                // for template string rendering: the nested template is rendered
                // as string content. Returning the inner template directly lets
                // downstream fold paths (including the parser-TIR-backed route)
                // handle it as a nested template rather than failing on the
                // Coerced wrapper.
                if matches!(value.kind, ExpressionKind::Template(_)) {
                    return Ok(FoldResolvedExpression::Borrowed(value));
                }
                return Ok(FoldResolvedExpression::Borrowed(expression));
            }

            // Inner value was rewritten: rebuild the coerced wrapper with the
            // resolved inner value. Only allocate because the inner actually changed.
            let resolved_owned = resolved.into_owned();
            add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
            add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 1);
            Ok(FoldResolvedExpression::Owned(Box::new(Expression {
                kind: ExpressionKind::Coerced {
                    value: Box::new(resolved_owned),
                    to_type: *to_type,
                },
                ..expression.clone()
            })))
        }

        ExpressionKind::Runtime(rpn) => {
            fold_runtime_expression_with_bindings(expression, rpn, fold_context)
        }

        // All other expression kinds have no foldable bindings — borrow unchanged.
        _ => Ok(FoldResolvedExpression::Borrowed(expression)),
    }
}

/// Resolves fold bindings in a runtime RPN expression.
///
/// WHAT: substitutes foldable bindings inside RPN operand expressions and
///       attempts constant folding on the substituted result. Returns a borrowed
///       reference when no operand was substituted and folding did not produce
///       a new value.
/// WHY: RPN expressions in const template loops are the other main allocation
///      hot spot. When all operands are non-binding references or literals,
///      the expression passes through unchanged and should not be cloned.
fn fold_runtime_expression_with_bindings<'a>(
    expression: &'a Expression,
    rpn: &ExpressionRpn,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<FoldResolvedExpression<'a>, TemplateError> {
    let mut substituted = Vec::with_capacity(rpn.items.len());
    let mut any_substituted = false;

    for item in &rpn.items {
        let new_item = match item {
            ExpressionRpnItem::Operand(value) => {
                let resolved = resolve_fold_bindings_in_expression(value, fold_context)?;
                match resolved {
                    FoldResolvedExpression::Borrowed(_) => {
                        // Operand unchanged — push the original clone (operator
                        // nodes need owned items in the substituted Vec).
                        item.clone()
                    }
                    FoldResolvedExpression::Owned(owned) => {
                        any_substituted = true;
                        add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
                        ExpressionRpnItem::Operand(*owned)
                    }
                }
            }
            ExpressionRpnItem::Operator { .. } => item.clone(),
        };
        substituted.push(new_item);
    }

    // No operand was substituted and constant folding has nothing new to
    // evaluate — borrow the original expression unchanged.
    if !any_substituted {
        return Ok(FoldResolvedExpression::Borrowed(expression));
    }

    // At least one operand was substituted; attempt constant folding on the
    // updated RPN to see if the expression can be simplified further.
    // Folding consumes what it is given, and both non-folding outcomes below rebuild a runtime
    // node from the pre-fold items, so this caller keeps its own copy.
    add_ast_counter(AstCounter::ExpressionOperandClones, substituted.len());

    match constant_fold(substituted.clone(), fold_context.string_table) {
        Ok(mut stack) => {
            if stack.len() == 1
                && let Some(ExpressionRpnItem::Operand(folded)) = stack.pop()
            {
                add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
                add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 1);
                return Ok(FoldResolvedExpression::Owned(Box::new(folded)));
            }
            // Folding did not simplify to a single value; build a new Runtime
            // expression from the substituted RPN.
            add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
            add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 1);
            Ok(FoldResolvedExpression::Owned(Box::new(Expression {
                kind: ExpressionKind::Runtime(ExpressionRpn { items: substituted }),
                ..expression.clone()
            })))
        }

        Err(_) => {
            // Constant folding failed; build a new Runtime expression from the
            // substituted RPN so downstream sees the substituted operands.
            add_ast_counter(AstCounter::TemplateFoldExpressionCloneRequests, 1);
            add_ast_counter(AstCounter::TemplateFoldExpressionOwnedRewrites, 1);
            Ok(FoldResolvedExpression::Owned(Box::new(Expression {
                kind: ExpressionKind::Runtime(ExpressionRpn { items: substituted }),
                ..expression.clone()
            })))
        }
    }
}
