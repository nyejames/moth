//! Arm selection for prepared const-function matches.
//!
//! WHAT: tests the AST's resolved match patterns against one evaluated scrutinee and selects the
//!       arm body the frame machine runs next.
//! WHY: a const match must follow the runtime order exactly. The scrutinee is evaluated once,
//!      patterns are tested in authored order and a guard runs only after its own pattern
//!      matches. Pattern facts, captures and exhaustiveness all come from the typed AST, so this
//!      module owns no pattern grammar and no second exhaustiveness check.
//! MUST NOT: push frames, re-check exhaustiveness or evaluate an unselected arm body.

use super::{
    ConstValueId,
    instructions::{ConstInstruction, ConstMatchArm},
    labels,
    state::{ConstInterpreter, FrameBindings},
    storage::{LiveValue, Permission},
    values::unsupported_error,
};
use crate::compiler_frontend::ast::const_eval::{ConstantFoldError, fold_pattern_comparison};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::ExprId;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, PatternValue, PatternValueKind, RelationalPatternOp,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::rc::Rc;

impl ConstInterpreter<'_> {
    /// Return the body of the first arm whose pattern and guard accept the scrutinee, or the
    /// default body when none does.
    ///
    /// WHAT: `Ok(None)` means a handler transferred control, so no arm body runs.
    /// WHY: a matching pattern binds its captures before its guard runs, as the runtime does.
    ///      A capture left behind by a rejecting guard needs no removal: an arm's guard and body
    ///      can name only its own captures, which its pattern rebinds before either runs, and no
    ///      code outside the match can name a capture path.
    pub(super) fn select_match_arm(
        &mut self,
        scrutinee: ExprId,
        arms: &[ConstMatchArm],
        default: Option<&Rc<[ConstInstruction]>>,
        bindings: &mut FrameBindings,
    ) -> Result<Option<Rc<[ConstInstruction]>>, ConstantFoldError> {
        let Some(scrutinee) = self.evaluate_expression(scrutinee, bindings)? else {
            return Ok(None);
        };
        for arm in arms {
            if !self.pattern_matches(&arm.pattern, scrutinee, bindings)? {
                continue;
            }
            if let Some(guard) = arm.guard {
                let Some(accepted) = self.evaluate_bool(guard, bindings)? else {
                    return Ok(None);
                };
                if !accepted {
                    continue;
                }
            }
            return Ok(Some(Rc::clone(&arm.body)));
        }
        match default {
            Some(body) => Ok(Some(Rc::clone(body))),
            None => Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "a match the AST proved exhaustive selected no arm",
            )
            .into()),
        }
    }

    /// Test one pattern against the scrutinee, binding its captures when it matches.
    fn pattern_matches(
        &mut self,
        pattern: &MatchPattern,
        scrutinee: LiveValue,
        bindings: &mut FrameBindings,
    ) -> Result<bool, ConstantFoldError> {
        let scrutinee = self.transparent_live_value(scrutinee)?;
        let payload = stored_payload(self.constants, scrutinee.shape)?;
        match pattern {
            MatchPattern::OptionNone { .. } => Ok(matches!(payload, ConstValuePayload::OptionNone)),
            MatchPattern::OptionPresentCapture { binding_path, .. } => {
                if !matches!(payload, ConstValuePayload::OptionSome(_)) {
                    return Ok(false);
                }
                self.bind_capture(bindings, scrutinee, 0, *binding_path)?;
                Ok(true)
            }
            MatchPattern::ChoiceVariant { tag, captures, .. } => {
                let ConstValuePayload::Choice { tag: value_tag, .. } = payload else {
                    return Ok(false);
                };
                if value_tag != tag {
                    return Ok(false);
                }
                for capture in captures {
                    self.bind_capture(
                        bindings,
                        scrutinee,
                        capture.field_index,
                        capture.binding_path,
                    )?;
                }
                Ok(true)
            }
            MatchPattern::Literal(value) | MatchPattern::OptionValue { value, .. } => {
                self.present_value_matches(scrutinee, value, &Operator::Equality)
            }
            MatchPattern::Relational { op, value, .. } => {
                self.present_value_matches(scrutinee, value, &relational_operator(*op))
            }
        }
    }

    fn transparent_live_value(
        &mut self,
        mut value: LiveValue,
    ) -> Result<LiveValue, ConstantFoldError> {
        self.validate_live_value(value, self.budget.span)?;
        while matches!(
            stored_payload(self.constants, value.shape)?,
            ConstValuePayload::Coerced(_)
        ) {
            let place = self
                .state
                .storage
                .project(value, 0)?
                .ok_or_else(missing_capture)?;
            value = self.read_place(place, self.budget.span)?;
        }
        Ok(value)
    }

    fn bind_capture(
        &mut self,
        bindings: &mut FrameBindings,
        scrutinee: LiveValue,
        index: usize,
        path: crate::compiler_frontend::symbols::path_interner::PathId,
    ) -> Result<(), ConstantFoldError> {
        let place = self
            .state
            .storage
            .project(scrutinee, index)?
            .ok_or_else(missing_capture)?;
        let value = self.read_place(place, self.budget.span)?;
        self.bind_slot(bindings, path, value, Permission::ReadOnly)
    }

    /// Compare the scrutinee, or the present value of an option scrutinee, with a pattern value.
    fn present_value_matches(
        &mut self,
        scrutinee: LiveValue,
        pattern: &PatternValue,
        op: &Operator,
    ) -> Result<bool, ConstantFoldError> {
        let subject = match stored_payload(self.constants, scrutinee.shape)? {
            ConstValuePayload::OptionNone => return Ok(false),
            ConstValuePayload::OptionSome(_) => {
                let place = self
                    .state
                    .storage
                    .project(scrutinee, 0)?
                    .ok_or_else(missing_capture)?;
                let inner = self.read_place(place, self.budget.span)?;
                self.transparent_live_value(inner)?.shape
            }
            _ => scrutinee.shape,
        };
        let pattern_value = match pattern.kind {
            PatternValueKind::Constant(value) => value,
            // Pattern completion normally folds every authored literal, but the typed AST may
            // still hold an unfolded pattern expression. Folding it without bindings keeps a
            // pattern from ever reading a local.
            PatternValueKind::Runtime(expression) => self
                .fold_to_constant(expression, &mut FrameBindings::default())?
                .ok_or_else(|| {
                    unsupported_error(labels::MATCH_PATTERN, pattern.span, self.string_table)
                })?,
        };
        let pattern_value = transparent_value(self.constants, pattern_value)?;
        let matched = fold_pattern_comparison(
            stored_payload(self.constants, subject)?,
            stored_payload(self.constants, pattern_value)?,
            op,
            pattern.span,
            self.string_table,
        )
        .map_err(ConstantFoldError::Diagnostic)?;
        matched.ok_or_else(|| {
            unsupported_error(labels::MATCH_PATTERN, pattern.span, self.string_table)
        })
    }
}

fn missing_capture() -> InfrastructureFailure {
    InfrastructureFailure::internal(
        CompilerStage::ConstantEvaluation,
        "a const pattern capture names a field the value lacks",
    )
}

fn relational_operator(op: RelationalPatternOp) -> Operator {
    match op {
        RelationalPatternOp::LessThan => Operator::LessThan,
        RelationalPatternOp::LessThanOrEqual => Operator::LessThanOrEqual,
        RelationalPatternOp::GreaterThan => Operator::GreaterThan,
        RelationalPatternOp::GreaterThanOrEqual => Operator::GreaterThanOrEqual,
    }
}

/// Skip transparent `Coerced` wrappers, which change a value's static type but not its content.
fn transparent_value(
    constants: &ConstValueStore,
    mut value: ConstValueId,
) -> Result<ConstValueId, InfrastructureFailure> {
    while let ConstValuePayload::Coerced(inner) = stored_payload(constants, value)? {
        value = *inner;
    }
    Ok(value)
}

fn stored_payload(
    constants: &ConstValueStore,
    value: ConstValueId,
) -> Result<&ConstValuePayload, InfrastructureFailure> {
    constants.payload(value).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "a match operand is outside its constant owner",
        )
    })
}
