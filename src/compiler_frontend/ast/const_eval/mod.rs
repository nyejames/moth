//! Conservative constant folding over the phase-owned expression DAG.
//!
//! Operations execute their retained resolved domains against borrowed constant content.
//! Fold output stays producer-local until the complete root is accepted; refusal retains
//! the original root and never publishes an intermediate constant or expression row.
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueCandidate, ConstValueField, ConstValueId,
    ConstValuePayload, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ResolvedComparisonOperation, ResolvedNumericOperation, ResolvedOperation, ResolvedOperationKind,
};
use crate::compiler_frontend::ast::expressions::expression_store::*;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::expressions::expression_types::{
    FallibleHandling, ResolvedCastEvidence,
};
use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::casts::{BuiltinCastLiteral, apply_builtin_cast_policy};
use crate::compiler_frontend::compiler_errors::{CompilerError, InfrastructureErrors};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, CompilerDiagnostic, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticPayload, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::number::{NumberArithmeticError, NumberValue};
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericComparisonDomain, NumericOperator,
};
use crate::compiler_frontend::datatypes::numeric_power;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use std::borrow::Cow;

#[derive(Debug)]
pub(crate) enum ConstantFoldError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<InfrastructureErrors>),
}
#[derive(Debug)]
pub(crate) enum ConstantFoldOutcome {
    Folded(ExprId),
    NotConstant(ExprId),
    TextUnavailable {
        root: ExprId,
        diagnostic: CompilerDiagnostic,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstStringRequirement {
    EqualityComparison,
    CastOrParse,
    CompileTimeMapKey,
    DuplicateKeyValidation,
}
impl ConstStringRequirement {
    fn operation_name(self) -> &'static str {
        match self {
            Self::EqualityComparison => "string equality comparison",
            Self::CastOrParse => "string cast or parse",
            Self::CompileTimeMapKey => "compile-time map key",
            Self::DuplicateKeyValidation => "duplicate map-key validation",
        }
    }
}

impl From<CompilerDiagnostic> for ConstantFoldError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ConstantFoldError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for ConstantFoldError {
    fn from(error: CompilerError) -> Self {
        ConstantFoldError::Infrastructure(error.into())
    }
}

impl From<Box<InfrastructureErrors>> for ConstantFoldError {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        Self::Infrastructure(errors)
    }
}

impl From<TemplateError> for ConstantFoldError {
    fn from(error: TemplateError) -> Self {
        match error {
            TemplateError::Diagnostic(diagnostic) => ConstantFoldError::Diagnostic(diagnostic),
            TemplateError::Infrastructure(error) => ConstantFoldError::Infrastructure(error),
        }
    }
}

impl From<ExpressionConstructionFailure> for ConstantFoldError {
    fn from(error: ExpressionConstructionFailure) -> Self {
        match error {
            ExpressionConstructionFailure::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ExpressionConstructionFailure::Invariant(error) => error.into(),
        }
    }
}
impl From<ConstValueStoreError> for ConstantFoldError {
    fn from(error: ConstValueStoreError) -> Self {
        match error {
            ConstValueStoreError::Diagnostic(diagnostic) => Self::Diagnostic(diagnostic),
            ConstValueStoreError::Infrastructure(error) => Self::Infrastructure(error),
        }
    }
}

pub(crate) fn require_concrete_text(
    value: ExpressionView<'_>,
    constants: &ConstValueStore,
    requirement: ConstStringRequirement,
    strings: &mut StringTable,
) -> Result<Option<StringId>, CompilerDiagnostic> {
    let ExpressionOperationView::Constant(id) = value.operation() else {
        return Ok(None);
    };
    let Some(payload) = constants.payload(id) else {
        return Ok(None);
    };
    concrete_payload_text(payload, value.span(), requirement, strings)
}
fn concrete_payload_text(
    payload: &ConstValuePayload,
    span: Option<SourceSpan>,
    requirement: ConstStringRequirement,
    strings: &mut StringTable,
) -> Result<Option<StringId>, CompilerDiagnostic> {
    match payload {
        ConstValuePayload::String(ConstStringValue::Text(text)) => Ok(Some(*text)),
        ConstValuePayload::String(ConstStringValue::Pieces(pieces)) => {
            let mut text = String::new();
            for piece in pieces {
                match piece {
                    ConstStringPiece::Text(part) => text.push_str(strings.resolve(*part)),
                    ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                        let operation =
                            strings.get_or_intern(requirement.operation_name().to_owned());
                        return Err(CompilerDiagnostic::compile_time_evaluation_error(
                            CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
                            Some(operation),
                            span,
                        ));
                    }
                }
            }
            Ok(Some(strings.get_or_intern(text)))
        }
        _ => Ok(None),
    }
}

use crate::compiler_frontend::ast::expressions::expression_rewrite::rewrite_expression_children;
/// Which constant boundary one fold serves; it decides how far composite constructors fold.
#[derive(Clone, Copy)]
enum FoldBoundary<'a> {
    /// A runtime expression folds scalar operators, casts and fallible recovery only. Aggregate and
    /// coercion constructors stay structural, so the runtime expression keeps its shape.
    Runtime { constant_context: bool },
    /// A constant boundary (declaration or signature default) materialises every foldable constructor
    /// into the value store, using `types` for option and choice identity.
    Content { types: &'a TypeEnvironment },
}
impl<'a> FoldBoundary<'a> {
    fn constant_context(self) -> bool {
        matches!(
            self,
            FoldBoundary::Runtime {
                constant_context: true
            }
        )
    }
    fn folds_composites(self) -> bool {
        matches!(self, FoldBoundary::Content { .. })
    }
    /// The type environment that composite folds consult; only constant content boundaries carry one.
    fn types(self) -> Result<&'a TypeEnvironment, CompilerError> {
        match self {
            FoldBoundary::Content { types } => Ok(types),
            FoldBoundary::Runtime { .. } => Err(CompilerError::compiler_error(
                "Composite constant folding requires a constant content boundary.",
            )),
        }
    }
}

/// Executes against an immutable expression prefix and a discardable constant candidate.
struct FoldContext<'a, 'prefix, F> {
    values: &'a mut ConstValueCandidate<'prefix>,
    strings: &'a mut StringTable,
    lookup: F,
    boundary: FoldBoundary<'a>,
    scope: Option<&'a ScopeContext>,
    /// Expression rows the walk reads. The fold never mutates them.
    expressions: &'a AstExpressionBuilder,
    text_unavailable: Option<CompilerDiagnostic>,
}

/// One step of the explicit fold walk. Frames hold ids and a result base, so depth costs heap
/// space rather than Rust stack frames.
#[derive(Clone, Copy)]
enum FoldStep {
    /// Visit one expression: settle it now, or park it beneath the children it needs.
    Enter(ExprId),
    /// Combine the child results from `base` onwards into this expression's value.
    Complete { expression: ExprId, base: usize },
    /// Stage a cast whose source has settled. Its policy is checked before visiting the source.
    CompleteCast {
        expression: ExprId,
        base: usize,
        policy: BuiltinCastPolicyId,
    },
}

/// Entry can settle a root without allocating work or result slots.
enum FoldEntry {
    Settled(Option<ConstValueId>),
    Deferred,
}

/// Work stack and result slots for one fold walk. A settled child pushes exactly one slot,
/// so the children of a parked node are the last `n` slots from its `base`.
#[derive(Default)]
struct FoldWalk {
    steps: Vec<FoldStep>,
    results: Vec<Option<ConstValueId>>,
}

impl FoldWalk {
    /// Parks `step` beneath the child ids. Reverse only the newly pushed steps, so the first
    /// authored child is entered first without a separate child buffer.
    fn defer(&mut self, step: FoldStep, children: impl IntoIterator<Item = ExprId>) {
        self.steps.push(step);
        let child_start = self.steps.len();
        self.steps.extend(children.into_iter().map(FoldStep::Enter));
        self.steps[child_start..].reverse();
    }
}
impl<F: FnMut(PathId) -> Option<ConstValueId>> FoldContext<'_, '_, F> {
    fn insert(
        &mut self,
        expression: ExpressionView<'_>,
        payload: ConstValuePayload,
        kind: ConstValueKind,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let metadata = ConstValueStore::metadata_from_view(expression, kind)?;
        Ok(Some(self.values.insert_value(metadata, payload)?))
    }

    /// Labels a numeric failure with the #Config inputs its operand constants retained. Each numeric
    /// operator labels its own operands, as the baseline did.
    fn label_numeric_failure(
        &mut self,
        error: ConstantFoldError,
        operands: &[ConstValueId],
    ) -> ConstantFoldError {
        let ConstantFoldError::Diagnostic(mut diagnostic) = error else {
            return error;
        };
        let provenances = operands
            .iter()
            .filter_map(|operand| self.values.metadata(*operand))
            .map(|metadata| &metadata.synthetic_interface_provenance)
            .collect::<Vec<_>>();
        match attach_source_config_origin_labels(
            &mut diagnostic,
            &provenances,
            self.strings,
            self.scope,
        ) {
            Ok(()) => ConstantFoldError::Diagnostic(diagnostic),
            Err(error) => error,
        }
    }

    /// Folds `root` and every expression it reaches. The walk is iterative, so nesting depth grows
    /// the heap work stack rather than the Rust stack. Children are entered in authored order, so
    /// diagnostics keep their order.
    fn fold_walk(&mut self, root: ExprId) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let expressions = self.expressions;
        let expression = expressions.view(root)?;
        let mut walk = FoldWalk::default();
        if let FoldEntry::Settled(value) = self.enter(&mut walk, expression)? {
            return Ok(value);
        }
        while let Some(step) = walk.steps.pop() {
            match step {
                FoldStep::Enter(id) => {
                    let expression = expressions.view(id)?;
                    if let FoldEntry::Settled(value) = self.enter(&mut walk, expression)? {
                        walk.results.push(value);
                    }
                }
                FoldStep::Complete { expression, base } => {
                    let view = expressions.view(expression)?;
                    let value = self.complete(view, &walk.results[base..])?;
                    walk.results.truncate(base);
                    walk.results.push(value);
                }
                FoldStep::CompleteCast {
                    expression,
                    base,
                    policy,
                } => {
                    let view = expressions.view(expression)?;
                    let value = self.complete_cast(view, policy, &walk.results[base..])?;
                    walk.results.truncate(base);
                    walk.results.push(value);
                }
            }
        }
        match walk.results.pop() {
            Some(value) => Ok(value),
            None => Err(CompilerError::compiler_error(
                "Constant fold walk finished without a root result.",
            )
            .into()),
        }
    }

    /// Settles one expression, or parks it beneath the children that its completion combines.
    /// Each guard here mirrors the matching arm in [`Self::complete`], so a parked node always
    /// reaches the arm it was parked for. The guards read only `Copy` boundary state.
    fn enter(
        &mut self,
        walk: &mut FoldWalk,
        expression: ExpressionView<'_>,
    ) -> Result<FoldEntry, ConstantFoldError> {
        use ExpressionOperationView as Operation;
        let type_id = match expression.success_shape() {
            SuccessShape::One(id) => id,
            _ => return Ok(FoldEntry::Settled(None)),
        };
        let base = walk.results.len();
        match expression.operation() {
            Operation::Constant(id) => {
                if self.values.metadata(id).map(|metadata| metadata.type_id) != Some(type_id) {
                    return Err(CompilerError::compiler_error(
                        "Constant expression disagrees with content type.",
                    )
                    .into());
                }
                let value = self.values.with_occurrence_provenance(expression, id)?;
                return Ok(FoldEntry::Settled(Some(value)));
            }
            Operation::Reference(path) => {
                let Some(id) = (self.lookup)(path) else {
                    return Ok(FoldEntry::Settled(None));
                };
                if self.values.metadata(id).map(|metadata| metadata.type_id) != Some(type_id) {
                    return Err(CompilerError::compiler_error(
                        "Constant binding disagrees with reference type.",
                    )
                    .into());
                }
                let value = self.values.with_occurrence_provenance(expression, id)?;
                return Ok(FoldEntry::Settled(Some(value)));
            }
            Operation::NumericUnary { operand, .. } | Operation::LogicalNot { operand } => {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    [operand.id()],
                );
            }
            Operation::NumericBinary { left, right, .. }
            | Operation::Compare { left, right, .. }
            | Operation::LogicalBinary { left, right, .. } => {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    [left.id(), right.id()],
                );
            }
            Operation::Range { start, end, .. } if self.boundary.folds_composites() => {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    [start.id(), end.id()],
                );
            }
            Operation::Coerce {
                value,
                operation: CoercionOperation::ToType(_),
            } if self.boundary.folds_composites() => {
                // Both pre-checks run before visiting the child.
                self.boundary.types()?;
                value.value_type_id()?;
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    [value.id()],
                );
            }
            Operation::Collection(items) if self.boundary.folds_composites() => {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    items.iter().map(|item| item.id()),
                );
            }
            Operation::Struct { fields } if self.boundary.folds_composites() => {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    fields.iter().map(|field| field.value().id()),
                );
            }
            Operation::Choice { fields, .. } if self.boundary.folds_composites() => {
                // The nominal type check runs before visiting the fields.
                self.boundary.types()?;
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    fields.iter().map(|field| field.value().id()),
                );
            }
            Operation::Cast { .. } => return self.enter_cast(walk, expression),
            Operation::ValueBlock(ValueBlock::Catch(block))
                if matches!(block.handler, FallibleHandling::Handler { .. }) =>
            {
                return self.enter_cast(walk, expression);
            }
            Operation::HandleFallible { value, .. } => {
                // A tail step: the wrapped value's result is the result of this expression.
                walk.steps.push(FoldStep::Enter(value.id()));
            }
            _ => {
                // Runtime parents still expose statically invalid child arithmetic.
                let child_start = walk.steps.len();
                expression.visit_children(|child| walk.steps.push(FoldStep::Enter(child.id())));
                if walk.steps.len() == child_start {
                    return Ok(FoldEntry::Settled(self.complete(expression, &[])?));
                }
                // Reverse completion with the children to park it beneath authored-order visits.
                walk.steps.push(FoldStep::Complete {
                    expression: expression.id(),
                    base,
                });
                walk.steps[child_start..].reverse();
            }
        }
        Ok(FoldEntry::Deferred)
    }

    /// Parks a cast beneath its source once its policy is known. A cast that cannot fold here
    /// leaves its source unvisited.
    fn enter_cast(
        &mut self,
        walk: &mut FoldWalk,
        expression: ExpressionView<'_>,
    ) -> Result<FoldEntry, ConstantFoldError> {
        let Some((source, facts)) = cast_operands(expression)? else {
            return Ok(FoldEntry::Settled(None));
        };
        let Some(policy) = self.cast_policy(expression, facts)? else {
            return Ok(FoldEntry::Settled(None));
        };
        let base = walk.results.len();
        walk.defer(
            FoldStep::CompleteCast {
                expression: expression.id(),
                base,
                policy,
            },
            [source.id()],
        );
        Ok(FoldEntry::Deferred)
    }

    /// Chooses the builtin policy for a cast, or refuses before its source is visited. `Ok(None)`
    /// leaves the cast unfolded and unvisited. A constant context reports non-foldable evidence.
    fn cast_policy(
        &self,
        original: ExpressionView<'_>,
        facts: &CastFacts,
    ) -> Result<Option<BuiltinCastPolicyId>, ConstantFoldError> {
        #[cfg(feature = "benchmark_counters")]
        increment_frontend_counter(FrontendCounter::CensusCastFoldAttempts);
        match facts.evidence {
            ResolvedCastEvidence::Builtin { policy } if policy.is_const_foldable() => {
                Ok(Some(policy))
            }
            ref evidence => {
                if !self.boundary.constant_context() {
                    return Ok(None);
                }
                let reason = match evidence {
                    ResolvedCastEvidence::Builtin { .. } => {
                        InvalidCastReason::BuiltinEvidenceNotConstFoldable
                    }
                    ResolvedCastEvidence::UserDefined { .. } => {
                        InvalidCastReason::UserDefinedEvidenceNotConstFoldable
                    }
                    ResolvedCastEvidence::GenericBound { .. } => {
                        InvalidCastReason::GenericBoundEvidenceNotConstFoldable
                    }
                };
                Err(CompilerDiagnostic::invalid_cast(
                    reason,
                    Some(facts.source_type_id),
                    Some(facts.target_type_id),
                    original.span(),
                )
                .into())
            }
        }
    }

    /// Combines the settled children of an expression that [`Self::enter`] parked. `children`
    /// holds one slot per child in authored order, and a `None` slot means that child did not fold.
    fn complete(
        &mut self,
        expression: ExpressionView<'_>,
        children: &[Option<ConstValueId>],
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        use ExpressionOperationView as Operation;
        let payload = match expression.operation() {
            Operation::NumericUnary { facts, .. } => {
                let [value] = child_slots::<1>(children)?;
                let Some(value) = value else {
                    return Ok(None);
                };
                match numeric_negation_payload(self.values, self.strings, value, facts) {
                    Ok(payload) => payload,
                    Err(error) => return Err(self.label_numeric_failure(error, &[value])),
                }
            }
            Operation::LogicalNot { .. } => {
                let [operand] = child_slots::<1>(children)?;
                let Some(operand) = operand else {
                    return Ok(None);
                };
                let Some(ConstValuePayload::Bool(value)) = self.values.payload(operand) else {
                    return Ok(None);
                };
                ConstValuePayload::Bool(!value)
            }
            Operation::NumericBinary { facts, .. } | Operation::Compare { facts, .. } => {
                let [left, right] = child_slots::<2>(children)?;
                let (Some(left), Some(right)) = (left, right) else {
                    return Ok(None);
                };
                let left_payload = numeric_operand(self.values, left);
                let right_payload = numeric_operand(self.values, right);
                match binary_payload(
                    left_payload,
                    right_payload,
                    facts,
                    expression.span(),
                    self.strings,
                ) {
                    Ok(PayloadFoldOutcome::Folded(payload)) => payload,
                    Ok(PayloadFoldOutcome::NotConstant) => return Ok(None),
                    Ok(PayloadFoldOutcome::TextUnavailable(diagnostic)) => {
                        self.text_unavailable.get_or_insert(diagnostic);
                        return Ok(None);
                    }
                    Err(error) if matches!(facts.kind, ResolvedOperationKind::Numeric(_)) => {
                        return Err(self.label_numeric_failure(error, &[left, right]));
                    }
                    Err(error) => return Err(error),
                }
            }
            Operation::LogicalBinary { operation, .. } => {
                let [left, right] = child_slots::<2>(children)?;
                let (Some(left), Some(right)) = (left, right) else {
                    return Ok(None);
                };
                let (Some(ConstValuePayload::Bool(left)), Some(ConstValuePayload::Bool(right))) =
                    (self.values.payload(left), self.values.payload(right))
                else {
                    return Ok(None);
                };
                ConstValuePayload::Bool(match operation {
                    LogicalOperator::And => *left && *right,
                    LogicalOperator::Or => *left || *right,
                })
            }
            Operation::Range { .. } if self.boundary.folds_composites() => {
                let [start, end] = child_slots::<2>(children)?;
                let (Some(start), Some(end)) = (start, end) else {
                    return Ok(None);
                };
                return self.insert(
                    expression,
                    ConstValuePayload::Range { start, end },
                    ConstValueKind::Composite,
                );
            }
            Operation::Coerce {
                value: source,
                operation: CoercionOperation::ToType(target),
            } if self.boundary.folds_composites() => {
                let types = self.boundary.types()?;
                let source_type = source.value_type_id()?;
                let [value] = child_slots::<1>(children)?;
                let Some(value) = value else {
                    return Ok(None);
                };
                let payload = if types.option_inner_type(target) == Some(source_type) {
                    ConstValuePayload::OptionSome(value)
                } else {
                    ConstValuePayload::Coerced(value)
                };
                return self.insert(expression, payload, ConstValueKind::Composite);
            }
            Operation::Collection(_) if self.boundary.folds_composites() => {
                let Some(values) = children.iter().copied().collect::<Option<Vec<_>>>() else {
                    return Ok(None);
                };
                return self.insert(
                    expression,
                    ConstValuePayload::Collection(values),
                    ConstValueKind::Composite,
                );
            }
            Operation::Struct { fields } if self.boundary.folds_composites() => {
                let Some(fields) = record_fields(fields, children) else {
                    return Ok(None);
                };
                return self.insert(
                    expression,
                    ConstValuePayload::Record(fields),
                    ConstValueKind::Composite,
                );
            }
            Operation::Choice { tag, fields } if self.boundary.folds_composites() => {
                let types = self.boundary.types()?;
                let SuccessShape::One(type_id) = expression.success_shape() else {
                    return Err(CompilerError::compiler_error(
                        "Folded choice lost its success type.",
                    )
                    .into());
                };
                let Some(fields) = record_fields(fields, children) else {
                    return Ok(None);
                };
                let nominal_path = *types.nominal_path(type_id).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "Folded choice has no canonical nominal identity.",
                    )
                })?;
                let payload = ConstValuePayload::Choice {
                    nominal_path,
                    tag: tag as usize,
                    fields,
                };
                return self.insert(expression, payload, ConstValueKind::Composite);
            }
            _ => return Ok(None),
        };
        self.insert(expression, payload, ConstValueKind::Literal)
    }

    /// Completes a cast once its source has folded to the single child slot.
    fn complete_cast(
        &mut self,
        expression: ExpressionView<'_>,
        policy: BuiltinCastPolicyId,
        children: &[Option<ConstValueId>],
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let [value] = child_slots::<1>(children)?;
        let Some(value) = value else {
            return Ok(None);
        };
        let Some((source, facts)) = cast_operands(expression)? else {
            return Err(
                CompilerError::compiler_error("Cast completion lost its cast operands.").into(),
            );
        };
        self.cast_complete(expression, source, facts, policy, value)
    }

    /// Stages the folded result of a cast whose source produced `value`. Runs only after the
    /// source folded, so the policy and evidence checks have already passed.
    fn cast_complete(
        &mut self,
        original: ExpressionView<'_>,
        source: ExpressionView<'_>,
        facts: &CastFacts,
        policy: BuiltinCastPolicyId,
        value: ConstValueId,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let payload = self
            .values
            .payload(value)
            .expect("cast fold returned an owned value");
        let literal = match payload {
            ConstValuePayload::Bool(value) => BuiltinCastLiteral::Bool(*value),
            ConstValuePayload::FixedScalar(value) => BuiltinCastLiteral::Fixed(*value),
            ConstValuePayload::Number(value) => BuiltinCastLiteral::Number(value.clone()),
            ConstValuePayload::Char(value) => BuiltinCastLiteral::Char(*value),
            ConstValuePayload::String(_) => match concrete_payload_text(
                payload,
                source.span(),
                ConstStringRequirement::CastOrParse,
                self.strings,
            ) {
                Ok(Some(value)) => {
                    BuiltinCastLiteral::String(self.strings.resolve(value).to_owned())
                }
                Ok(None) => return Ok(None),
                Err(diagnostic) if self.boundary.constant_context() => {
                    return Err(diagnostic.into());
                }
                Err(diagnostic) => {
                    self.text_unavailable.get_or_insert(diagnostic);
                    return Ok(None);
                }
            },
            _ => return Ok(None),
        };
        let literal = apply_builtin_cast_policy(policy, &literal).map_err(|_| {
            CompilerDiagnostic::invalid_cast(
                InvalidCastReason::BuiltinCastFailedInConst,
                Some(facts.source_type_id),
                Some(facts.target_type_id),
                original.span(),
            )
        })?;
        let payload = match literal {
            BuiltinCastLiteral::Bool(value) => ConstValuePayload::Bool(value),
            BuiltinCastLiteral::Fixed(value) => ConstValuePayload::FixedScalar(value),
            BuiltinCastLiteral::Number(value) => ConstValuePayload::Number(value),
            BuiltinCastLiteral::Char(value) => ConstValuePayload::Char(value),
            BuiltinCastLiteral::String(value) => {
                ConstValuePayload::String(ConstStringValue::Text(self.strings.get_or_intern(value)))
            }
            BuiltinCastLiteral::Error { .. } => return Ok(None),
        };
        // The success value carries its source's provenance, as the baseline cast did. The
        // occurrence (for example a Catch that also names a handler) must not add members.
        let source_provenance = self
            .values
            .metadata(value)
            .map(|source| source.synthetic_interface_provenance.clone())
            .unwrap_or_default();
        let mut metadata = ConstValueStore::metadata_from_view(original, ConstValueKind::Literal)?;
        metadata.type_id = facts.target_type_id;
        metadata.synthetic_interface_provenance = source_provenance.clone();
        let value = self.values.insert_value(metadata, payload)?;
        if facts.requires_optional_wrap_after_cast {
            let mut wrapper =
                ConstValueStore::metadata_from_view(original, ConstValueKind::Composite)?;
            wrapper.synthetic_interface_provenance = source_provenance;
            Ok(Some(self.values.insert_value(
                wrapper,
                ConstValuePayload::OptionSome(value),
            )?))
        } else {
            Ok(Some(value))
        }
    }
}

/// Reads the settled child slots of a node with a fixed child count. A mismatch means the walk
/// and the completion disagree about the node's shape, which is a compiler bug.
fn child_slots<const N: usize>(
    children: &[Option<ConstValueId>],
) -> Result<[Option<ConstValueId>; N], ConstantFoldError> {
    <[Option<ConstValueId>; N]>::try_from(children).map_err(|_| {
        ConstantFoldError::from(CompilerError::compiler_error(
            "Constant fold walk settled an unexpected child count.",
        ))
    })
}

/// Pairs settled field values with their names. Returns `None` when any field did not fold.
fn record_fields(
    fields: FieldListView<'_>,
    children: &[Option<ConstValueId>],
) -> Option<Vec<ConstValueField>> {
    let mut values = Vec::with_capacity(children.len());
    for (field, child) in fields.iter().zip(children) {
        values.push(ConstValueField {
            name: field.name(),
            value: (*child)?,
        });
    }
    Some(values)
}

/// The cast that a fold of `expression` completes: the cast itself, or the cast that a handled
/// Catch wraps. `None` means the expression is not a cast fold.
fn cast_operands(
    expression: ExpressionView<'_>,
) -> Result<Option<(ExpressionView<'_>, &CastFacts)>, CompilerError> {
    match expression.operation() {
        ExpressionOperationView::Cast { source, facts } => Ok(Some((source, facts))),
        ExpressionOperationView::ValueBlock(ValueBlock::Catch(block))
            if matches!(block.handler, FallibleHandling::Handler { .. }) =>
        {
            let handled = expression.bind(block.handled_value)?;
            match handled.operation() {
                ExpressionOperationView::Cast { source, facts } => Ok(Some((source, facts))),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

enum PayloadFoldOutcome {
    Folded(ConstValuePayload),
    NotConstant,
    TextUnavailable(CompilerDiagnostic),
}
/// A numeric coercion is transparent to arithmetic and comparison, so a `Coerced` operand
/// reads as the scalar it wraps.
fn numeric_operand<'a>(
    values: &'a ConstValueCandidate<'_>,
    mut id: ConstValueId,
) -> &'a ConstValuePayload {
    loop {
        let payload = values.payload(id).expect("fold returned an owned operand");
        let ConstValuePayload::Coerced(inner) = payload else {
            return payload;
        };
        id = *inner;
    }
}

/// Computes a numeric negation from its operand payload. The caller labels any failure from the operand.
fn numeric_negation_payload(
    values: &ConstValueCandidate<'_>,
    strings: &mut StringTable,
    value: ConstValueId,
    facts: &ResolvedOperation,
) -> Result<ConstValuePayload, ConstantFoldError> {
    let value = values
        .payload(value)
        .expect("staged fold returned an owned value");
    let mut context = NumericFoldFailure {
        operator: &facts.operator,
        span: facts.span,
        string_table: strings,
    };
    let ResolvedOperationKind::Numeric(numeric) = facts.kind else {
        return Err(
            CompilerError::compiler_error("Numeric unary operation lacks numeric facts.").into(),
        );
    };
    match numeric.domain {
        NumericScalar::Number(_) => {
            let ConstValuePayload::Number(value) = value else {
                return Err(CompilerError::compiler_error(
                    "Dec negation received a non-Dec operand.",
                )
                .into());
            };
            Ok(ConstValuePayload::Number(value.negated()))
        }
        domain if domain.is_integer() => {
            let value = integer_value_from_payload(value).ok_or_else(|| {
                CompilerError::compiler_error("Integer negation received a non-integer operand.")
            })?;
            let Some(value) = value.checked_neg() else {
                return integer_overflow_error(&mut context);
            };
            let (minimum, maximum) = domain.integer_range().expect("integer domain has bounds");
            if !(minimum..=maximum).contains(&value) {
                return integer_overflow_error(&mut context);
            }
            Ok(integer_result_payload(domain, value))
        }
        domain => {
            let precision = domain
                .binary_float_precision()
                .ok_or_else(|| CompilerError::compiler_error("Negation lacks a numeric domain."))?;
            let value = float_value_from_payload(value).ok_or_else(|| {
                CompilerError::compiler_error("Float negation received a non-float operand.")
            })?;
            Ok(float_result_payload(
                domain,
                checked_float_result(-value, precision, &mut context)?,
            ))
        }
    }
}

fn binary_payload(
    left: &ConstValuePayload,
    right: &ConstValuePayload,
    facts: &ResolvedOperation,
    span: Option<SourceSpan>,
    strings: &mut StringTable,
) -> Result<PayloadFoldOutcome, ConstantFoldError> {
    if let ResolvedOperationKind::Numeric(numeric) = &facts.kind {
        let mut context = NumericFoldFailure {
            operator: &facts.operator,
            span: facts.span,
            string_table: strings,
        };
        return Ok(PayloadFoldOutcome::Folded(fold_numeric_binary(
            left,
            right,
            numeric,
            &mut context,
        )?));
    }
    if let ResolvedOperationKind::Comparison(comparison) = &facts.kind {
        return Ok(
            match fold_numeric_comparison(left, right, &facts.operator, comparison) {
                Some(value) => PayloadFoldOutcome::Folded(ConstValuePayload::Bool(value)),
                None => PayloadFoldOutcome::NotConstant,
            },
        );
    }
    let value = match (left, right) {
        (ConstValuePayload::Bool(left), ConstValuePayload::Bool(right)) => match facts.operator {
            Operator::And => Some(*left && *right),
            Operator::Or => Some(*left || *right),
            Operator::Equality => Some(left == right),
            Operator::NotEqual => Some(left != right),
            _ => invalid_operator_for_compile_time_type(&facts.operator, strings, facts.span)?,
        },
        (ConstValuePayload::FixedScalar(left), ConstValuePayload::FixedScalar(right))
            if left.scalar() == FixedScalar::Byte && right.scalar() == FixedScalar::Byte =>
        {
            left.as_u64()
                .zip(right.as_u64())
                .and_then(|(left, right)| comparison_result(&facts.operator, left.cmp(&right)))
        }
        (ConstValuePayload::String(_), ConstValuePayload::String(_)) => {
            if !matches!(facts.operator, Operator::Equality | Operator::NotEqual) {
                invalid_operator_for_compile_time_type::<()>(&facts.operator, strings, facts.span)?;
            }
            let requirement = ConstStringRequirement::EqualityComparison;
            let left = concrete_payload_text(left, span, requirement, strings);
            let right = concrete_payload_text(right, span, requirement, strings);
            match (left, right) {
                (Ok(Some(left)), Ok(Some(right))) => {
                    Some(if matches!(facts.operator, Operator::Equality) {
                        left == right
                    } else {
                        left != right
                    })
                }
                (Err(diagnostic), _) | (_, Err(diagnostic)) => {
                    return Ok(PayloadFoldOutcome::TextUnavailable(diagnostic));
                }
                _ => None,
            }
        }
        _ => None,
    };
    Ok(match value {
        Some(value) => PayloadFoldOutcome::Folded(ConstValuePayload::Bool(value)),
        None => PayloadFoldOutcome::NotConstant,
    })
}

pub(crate) fn constant_fold(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    scope: Option<&ScopeContext>,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusConstantFoldEntries);
    let outcome = fold_root(
        root,
        builder,
        constants,
        strings,
        FoldBoundary::Runtime {
            constant_context: false,
        },
        scope,
        |_| None,
    )?;
    #[cfg(feature = "benchmark_counters")]
    match &outcome {
        ConstantFoldOutcome::Folded(_) => {
            increment_frontend_counter(FrontendCounter::CensusFoldOutcomeFolded)
        }
        ConstantFoldOutcome::NotConstant(_) => {
            increment_frontend_counter(FrontendCounter::CensusFoldOutcomeRuntime)
        }
        ConstantFoldOutcome::TextUnavailable { .. } => {
            increment_frontend_counter(FrontendCounter::CensusFoldOutcomeTextUnavailable)
        }
    }
    Ok(outcome)
}

pub(crate) fn fold_compile_time_expression(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    constant_context: bool,
) -> Result<ExprId, ConstantFoldError> {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusFoldCompileTimeExpressionEntries);
    Ok(outcome_root(fold_root(
        root,
        builder,
        constants,
        strings,
        FoldBoundary::Runtime { constant_context },
        None,
        |_| None,
    )?))
}

/// Scope and failure policy for one loop-binding substitution.
#[derive(Clone, Copy)]
pub(crate) struct BindingFold<'a> {
    scope: Option<&'a ScopeContext>,
    speculative: bool,
}

impl<'a> BindingFold<'a> {
    /// A rewritten operator whose fold fails with a diagnostic stays unfolded, so runtime
    /// lowering evaluates it. Baseline template folding treated such failures as speculative.
    pub(crate) fn speculative(scope: Option<&'a ScopeContext>) -> Self {
        Self {
            scope,
            speculative: true,
        }
    }

    /// A fold failure is returned to the caller.
    pub(crate) fn strict(scope: Option<&'a ScopeContext>) -> Self {
        Self {
            scope,
            speculative: false,
        }
    }
}

/// Substitute loop bindings without erasing coercion, template or runtime structure.
pub(crate) fn fold_expression_with_bindings(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    folding: BindingFold<'_>,
    mut lookup: impl FnMut(PathId) -> Option<ExprId>,
) -> Result<ExprId, ConstantFoldError> {
    substitute_bindings(
        root,
        builder,
        constants,
        strings,
        folding,
        &mut lookup,
        &mut rustc_hash::FxHashMap::default(),
    )
}
/// One step of the binding substitution walk. Frames hold ids and a result base only.
#[derive(Clone, Copy)]
enum SubstituteStep {
    /// Substitute one expression, or settle it from the memo or a reference binding.
    Enter(ExprId),
    /// Rewrite `expression` once its children have pushed their pairs from `base` onwards.
    Complete {
        expression: ExprId,
        base: usize,
        /// Whether a speculative fold may run on the rewritten row.
        can_fold: bool,
    },
}

/// Work stack and results for one substitution walk. A settled expression pushes one
/// `(original, replacement)` pair, so a parent's pairs are the slice from its `base`.
#[derive(Default)]
struct SubstituteWalk {
    steps: Vec<SubstituteStep>,
    results: Vec<(ExprId, ExprId)>,
    children: Vec<ExprId>,
}

/// Rewrites bindings under `root` and speculatively folds rewritten constant parents. The walk is
/// iterative, so nesting depth grows the heap work stack rather than the Rust stack. Children are
/// visited in authored order and memoised when their substitution result settles.
fn substitute_bindings(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    folding: BindingFold<'_>,
    lookup: &mut impl FnMut(PathId) -> Option<ExprId>,
    rewritten: &mut rustc_hash::FxHashMap<ExprId, ExprId>,
) -> Result<ExprId, ConstantFoldError> {
    let mut walk = SubstituteWalk::default();
    walk.steps.push(SubstituteStep::Enter(root));
    while let Some(step) = walk.steps.pop() {
        match step {
            SubstituteStep::Enter(expression) => {
                if let Some(&result) = rewritten.get(&expression) {
                    walk.results.push((expression, result));
                    continue;
                }
                let view = builder.view(expression)?;
                if let ExpressionOperationView::Reference(path) = view.operation() {
                    let result = lookup(path).unwrap_or(expression);
                    builder.view(result)?;
                    rewritten.insert(expression, result);
                    walk.results.push((expression, result));
                    continue;
                }
                let can_fold = matches!(
                    view.operation(),
                    ExpressionOperationView::NumericUnary { .. }
                        | ExpressionOperationView::NumericBinary { .. }
                        | ExpressionOperationView::LogicalNot { .. }
                        | ExpressionOperationView::LogicalBinary { .. }
                        | ExpressionOperationView::Compare { .. }
                        | ExpressionOperationView::Range { .. }
                );
                let base = walk.results.len();
                view.visit_children(|child| walk.children.push(child.id()));
                walk.steps.push(SubstituteStep::Complete {
                    expression,
                    base,
                    can_fold,
                });
                walk.steps
                    .extend(walk.children.drain(..).rev().map(SubstituteStep::Enter));
            }
            SubstituteStep::Complete {
                expression,
                base,
                can_fold,
            } => {
                let root = builder.own(expression);
                let replacements = builder.own_slice(&walk.results[base..]);
                let result = rewrite_expression_children(builder, root, replacements)?.id();
                let result = if result != expression && can_fold {
                    // A refused speculative fold leaves the operator for runtime lowering.
                    match constant_fold(result, builder, constants, strings, folding.scope) {
                        Ok(outcome) => outcome_root(outcome),
                        // Capacity exhaustion is never a speculative refusal: it must reach the module build.
                        Err(ConstantFoldError::Diagnostic(diagnostic))
                            if folding.speculative
                                && !diagnostic.is_compiler_capacity_exceeded() =>
                        {
                            result
                        }
                        Err(error) => return Err(error),
                    }
                } else {
                    result
                };
                rewritten.insert(expression, result);
                walk.results.truncate(base);
                walk.results.push((expression, result));
            }
        }
    }
    match walk.results.pop() {
        Some((_, result)) => Ok(result),
        None => Err(CompilerError::compiler_error(
            "Binding substitution finished without a root result.",
        )
        .into()),
    }
}

pub(crate) fn fold_expression_with_constants(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    types: &TypeEnvironment,
    lookup: impl FnMut(PathId) -> Option<ConstValueId>,
) -> Result<ExprId, ConstantFoldError> {
    Ok(outcome_root(fold_root(
        root,
        builder,
        constants,
        strings,
        FoldBoundary::Content { types },
        None,
        lookup,
    )?))
}
/// Fold one constant-boundary expression (a declaration or signature default).
///
/// Unlike [`constant_fold`], this materialises every foldable constructor (collections, records, choices,
/// ranges and coercions) so the stored constant carries its full shape.
pub(crate) fn fold_constant_content(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    scope: Option<&ScopeContext>,
    types: &TypeEnvironment,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    fold_root(
        root,
        builder,
        constants,
        strings,
        FoldBoundary::Content { types },
        scope,
        |_| None,
    )
}
fn outcome_root(outcome: ConstantFoldOutcome) -> ExprId {
    match outcome {
        ConstantFoldOutcome::Folded(root)
        | ConstantFoldOutcome::NotConstant(root)
        | ConstantFoldOutcome::TextUnavailable { root, .. } => root,
    }
}
fn fold_root(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    boundary: FoldBoundary<'_>,
    scope: Option<&ScopeContext>,
    lookup: impl FnMut(PathId) -> Option<ConstValueId>,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    let (existing_constant, success, span, value_mode, classification, occurrence) = {
        let view = builder.view(root)?;
        let existing = match view.operation() {
            ExpressionOperationView::Constant(id) => Some(id),
            _ => None,
        };
        (
            existing,
            view.success_shape(),
            view.span(),
            view.value_mode(),
            view.classification(),
            view.occurrence_facts().cloned(),
        )
    };
    let view = builder.view(root)?;
    // Baseline fold_resolved_cast kept authored eligibility for every fold except a Catch with a Handler.
    let eligibility = match view.operation() {
        ExpressionOperationView::ValueBlock(ValueBlock::Catch(block))
            if matches!(block.handler, FallibleHandling::Handler { .. }) =>
        {
            None
        }
        _ => view
            .failure_facts()
            .map(ExpressionFailureFacts::folded_constant_eligibility)
            .filter(|facts| *facts != ExpressionFailureFacts::default()),
    };
    let cast_root = matches!(
        view.operation(),
        ExpressionOperationView::Cast { .. }
            | ExpressionOperationView::ValueBlock(ValueBlock::Catch(_))
    );
    let mut text_unavailable = None;
    let value = constants.stage_values(|values| {
        let mut context = FoldContext {
            values,
            strings,
            lookup,
            boundary,
            scope,
            expressions: &*builder,
            text_unavailable: None,
        };
        let value = context.fold_walk(view.id())?;
        text_unavailable = context.text_unavailable;
        Ok::<_, ConstantFoldError>(if text_unavailable.is_some() {
            None
        } else {
            value
        })
    })?;
    if let Some(diagnostic) = text_unavailable {
        return Ok(ConstantFoldOutcome::TextUnavailable { root, diagnostic });
    }
    let Some(value) = value else {
        return Ok(ConstantFoldOutcome::NotConstant(root));
    };
    if existing_constant == Some(value) {
        return Ok(ConstantFoldOutcome::Folded(root));
    }
    // A builtin cast result, including a Catch that resolves to one, carries only its source's
    // provenance. The handler named by a Catch occurrence is never part of the success value.
    let occurrence = match occurrence {
        Some(mut facts) if cast_root => {
            facts.provenance = constants
                .metadata(value)
                .map(|source| source.synthetic_interface_provenance.clone())
                .unwrap_or_default();
            Some(facts)
        }
        other => other,
    };
    // A folded constant cannot fail at runtime; it keeps only the authored eligibility of the
    // operation it replaces, never origins or witnesses.
    let input = CompletedExpressionInput {
        operation: ExpressionOp::Constant(value),
        success,
        error_type: None,
        span,
        value_mode,
        classification,
        failure: eligibility,
        occurrence,
    };
    Ok(ConstantFoldOutcome::Folded(
        builder.commit_replacement(input)?.id(),
    ))
}
fn attach_source_config_origin_labels(
    diagnostic: &mut CompilerDiagnostic,
    provenances: &[&SyntheticInterfaceProvenance],
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> Result<(), ConstantFoldError> {
    if !matches!(
        &diagnostic.payload,
        DiagnosticPayload::CompileTimeEvaluationError { .. }
    ) {
        return Ok(());
    }

    let mut input_names = provenances
        .iter()
        .flat_map(|provenance| provenance.source_config_member_names())
        .collect::<Vec<_>>();
    input_names.sort_unstable();
    input_names.dedup();
    if input_names.is_empty() {
        return Ok(());
    }

    let Some(scope_context) = scope_context else {
        return Err(CompilerError::compiler_error(
            "A numeric compile-time diagnostic retained source #Config provenance without its owning scope resolver.",
        )
        .into());
    };

    for input_name in input_names {
        let Some((origin, Some(label_span))) =
            scope_context.source_config_diagnostic_origin(input_name)
        else {
            return Err(CompilerError::compiler_error(format!(
                "A numeric compile-time diagnostic retained source #Config input '{input_name}' without a matching resolver origin and source anchor."
            ))
            .into());
        };
        diagnostic.labels.push(DiagnosticLabel::secondary(
            Some(label_span),
            Some(DiagnosticLabelMessage::ConfigInputOrigin {
                input_name: string_table.intern(input_name),
                origin,
            }),
        ));
    }

    Ok(())
}

fn compile_time_evaluation_diagnostic(
    reason: CompileTimeEvaluationErrorReason,
    operation: Option<String>,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> ConstantFoldError {
    let operation = operation.map(|operation| string_table.get_or_intern(operation));

    CompilerDiagnostic::compile_time_evaluation_error(reason, operation, span).into()
}

fn integer_overflow_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::IntegerOverflow,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
    ))
}

fn float_non_finite_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::FloatOverflow,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
    ))
}

/// Shared failure context for one numeric fold attempt.
///
/// WHAT: carries the authored operator, its span and the shared string table so both helpers
///       funnel typed numeric diagnostics through one value.
/// WHY: keeps the fold-function argument lists short while failures retain the same span and
///       operation identity that the standalone parameters provided.
struct NumericFoldFailure<'table, 'operator> {
    operator: &'operator Operator,
    span: Option<SourceSpan>,
    string_table: &'table mut StringTable,
}

fn checked_float_result(
    value: f64,
    precision: BinaryFloatPrecision,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<f64, ConstantFoldError> {
    // Every numeric float domain rounds its semantic result once before the finite-value check.
    let rounded = precision.round(value);
    if rounded.is_finite() {
        Ok(rounded)
    } else {
        float_non_finite_error(ctx)
    }
}

/// Folds one integer operation exactly in `i128` and checks the result against the operation
/// domain's inclusive `range`, so every fixed integer type shares one set of rules.
fn checked_int_binary_result(
    lhs: i128,
    rhs: i128,
    (minimum, maximum): (i128, i128),
    numeric_operator: NumericOperator,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<i128, ConstantFoldError> {
    let checked = match numeric_operator {
        NumericOperator::Add => lhs.checked_add(rhs),
        NumericOperator::Subtract => lhs.checked_sub(rhs),
        NumericOperator::Multiply => lhs.checked_mul(rhs),
        NumericOperator::IntegerDivide => {
            if rhs == 0 {
                return divide_by_zero_error(ctx);
            }

            lhs.checked_div(rhs)
        }
        NumericOperator::Remainder => {
            if rhs == 0 {
                return divide_by_zero_error(ctx);
            }

            // Remainder by -1 is zero even where the paired quotient overflows.
            if rhs == -1 {
                Some(0)
            } else {
                lhs.checked_rem(rhs)
            }
        }
        NumericOperator::Power => {
            if rhs < 0 {
                return exponent_operand_context_error(ctx);
            }

            // Trivial bases have exact answers even when the exponent cannot fit u32. Other
            // powers use checked exponentiation, whose work grows logarithmically with exponent.
            if lhs == 0 {
                Some(if rhs == 0 { 1 } else { 0 })
            } else if lhs == 1 {
                Some(1)
            } else if lhs == -1 {
                Some(if rhs % 2 == 0 { 1 } else { -1 })
            } else {
                let Ok(exponent) = u32::try_from(rhs) else {
                    return integer_overflow_error(ctx);
                };
                lhs.checked_pow(exponent)
            }
        }
        NumericOperator::Divide | NumericOperator::Negate => {
            return Err(CompilerError::compiler_error(format!(
                "Integer folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };

    let Some(value) = checked else {
        return integer_overflow_error(ctx);
    };
    if !(minimum..=maximum).contains(&value) {
        return integer_overflow_error(ctx);
    }

    Ok(value)
}

fn divide_by_zero_error<T>(ctx: &mut NumericFoldFailure<'_, '_>) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::DivideByZero,
        None,
        ctx.string_table,
        ctx.span,
    ))
}

fn exponent_operand_context_error<T>(
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::InvalidExponent,
        Some(ctx.operator.to_str().to_string()),
        ctx.string_table,
        ctx.span,
    ))
}

fn invalid_operator_for_compile_time_type<T>(
    op: &Operator,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<T, ConstantFoldError> {
    Err(compile_time_evaluation_diagnostic(
        CompileTimeEvaluationErrorReason::InvalidOperatorForType,
        Some(op.to_str().to_string()),
        string_table,
        span,
    ))
}

fn integer_value_from_payload(expression: &ConstValuePayload) -> Option<i128> {
    match expression {
        ConstValuePayload::FixedScalar(value) => match value.scalar().class() {
            // The `u64` payload widens exactly into `i128`, so `U64` values above
            // `i64::MAX` keep their full unsigned value through checked folding and
            // comparison without a lossy intermediate.
            FixedScalarClass::SignedInteger => value.as_i64().map(i128::from),
            FixedScalarClass::UnsignedInteger => value.as_u64().map(i128::from),
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => None,
        },
        _ => None,
    }
}

fn float_value_from_payload(expression: &ConstValuePayload) -> Option<f64> {
    match expression {
        ConstValuePayload::FixedScalar(value) => value.as_f64(),
        _ => None,
    }
}

fn float_operand_in_domain(
    expression: &ConstValuePayload,
    conversion: Option<NumericScalar>,
) -> Option<f64> {
    if let Some(target) = conversion {
        let precision = target.binary_float_precision()?;
        if let Some(integer) = integer_value_from_payload(expression) {
            return Some(precision.round_integer(integer));
        }
        return float_value_from_payload(expression).map(|value| precision.round(value));
    }

    float_value_from_payload(expression)
}

fn integer_result_payload(domain: NumericScalar, value: i128) -> ConstValuePayload {
    match domain {
        NumericScalar::Fixed(scalar) => match scalar.class() {
            FixedScalarClass::SignedInteger => {
                // The checked domain range immediately precedes construction.
                let value = i64::try_from(value).expect("signed fixed result fits its i64 carrier");
                let value = FixedScalarValue::signed(scalar, value)
                    .expect("checked result fits the signed fixed domain");
                ConstValuePayload::FixedScalar(value)
            }
            FixedScalarClass::UnsignedInteger => {
                // The checked domain range immediately precedes construction.
                let value =
                    u64::try_from(value).expect("unsigned fixed result fits its u64 carrier");
                let value = FixedScalarValue::unsigned(scalar, value)
                    .expect("checked result fits the unsigned fixed domain");
                ConstValuePayload::FixedScalar(value)
            }
            FixedScalarClass::BinaryFloat | FixedScalarClass::Octet => {
                unreachable!("integer result domain must be an integer scalar")
            }
        },
        NumericScalar::Number(_) => unreachable!("Number results use the exact Number constructor"),
    }
}

fn float_result_payload(domain: NumericScalar, value: f64) -> ConstValuePayload {
    match domain {
        NumericScalar::Fixed(scalar) if scalar.class() == FixedScalarClass::BinaryFloat => {
            let value = FixedScalarValue::binary_float(scalar, value)
                .expect("rounded finite result fits the fixed float domain");
            ConstValuePayload::FixedScalar(value)
        }
        NumericScalar::Fixed(_) | NumericScalar::Number(_) => {
            unreachable!("float result domain must be a binary float scalar")
        }
    }
}

fn number_operand_in_domain(
    expression: &ConstValuePayload,
    scale: NumberScale,
    conversion: Option<NumericScalar>,
) -> Option<Cow<'_, NumberValue>> {
    match expression {
        ConstValuePayload::Number(value) if value.scale() == scale => Some(Cow::Borrowed(value)),
        // Exact fixed integers join the mixed-integer rule with their complete value,
        // including `U64` above `i64::MAX` and signed values below zero.
        ConstValuePayload::FixedScalar(_) if conversion == Some(NumericScalar::Number(scale)) => {
            integer_value_from_payload(expression)
                .map(|integer| Cow::Owned(NumberValue::from_integer(integer, scale)))
        }
        _ => None,
    }
}

fn fold_number_binary(
    lhs: &ConstValuePayload,
    rhs: &ConstValuePayload,
    numeric: &ResolvedNumericOperation,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<ConstValuePayload, ConstantFoldError> {
    let numeric_operator = numeric.operator;
    let NumericScalar::Number(scale) = numeric.domain else {
        return Err(CompilerError::compiler_error(
            "Dec folding received a non-Dec operation domain.",
        )
        .into());
    };

    let result = if numeric_operator == NumericOperator::Power {
        let ConstValuePayload::Number(base) = lhs else {
            return Err(CompilerError::compiler_error(
                "Dec power folding received a non-Dec base.",
            )
            .into());
        };
        // `Dec ^ I32` is the only Dec power row. The operator table already selected the
        // exact `I32` exponent, so any other exponent is a broken invariant rather than a
        // wider receiver repair.
        let ConstValuePayload::FixedScalar(exponent) = rhs else {
            return Err(CompilerError::compiler_error(
                "Dec power folding received a non-I32 exponent.",
            )
            .into());
        };
        if exponent.scalar() != FixedScalar::I32 {
            return Err(CompilerError::compiler_error(
                "Dec power folding received a non-I32 exponent.",
            )
            .into());
        }
        let Some(exponent) = exponent.as_i64() else {
            return Err(CompilerError::compiler_error(
                "Dec power folding received an unreadable I32 exponent.",
            )
            .into());
        };
        base.checked_power(exponent)
    } else {
        let Some(left_value) = number_operand_in_domain(lhs, scale, numeric.left_conversion) else {
            return Err(CompilerError::compiler_error(
                "Dec folding could not materialize the left operand in the result scale.",
            )
            .into());
        };
        let Some(right_value) = number_operand_in_domain(rhs, scale, numeric.right_conversion)
        else {
            return Err(CompilerError::compiler_error(
                "Dec folding could not materialize the right operand in the result scale.",
            )
            .into());
        };

        left_value
            .as_ref()
            .checked_binary(numeric_operator, right_value.as_ref())
    };

    let value = match result {
        Ok(value) => value,
        Err(NumberArithmeticError::DivideByZero) => {
            return divide_by_zero_error(ctx);
        }
        Err(NumberArithmeticError::InvalidExponent) => {
            return exponent_operand_context_error(ctx);
        }
        Err(NumberArithmeticError::InvalidOperation) => {
            return Err(CompilerError::compiler_error(format!(
                "Dec folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };

    Ok(ConstValuePayload::Number(value))
}

fn fold_numeric_binary(
    lhs: &ConstValuePayload,
    rhs: &ConstValuePayload,
    numeric: &ResolvedNumericOperation,
    ctx: &mut NumericFoldFailure<'_, '_>,
) -> Result<ConstValuePayload, ConstantFoldError> {
    let numeric_operator = numeric.operator;
    let domain = numeric.domain;
    if matches!(domain, NumericScalar::Number(_)) {
        return fold_number_binary(lhs, rhs, numeric, ctx);
    }

    if domain.is_integer() {
        let left_value = integer_value_from_payload(lhs)
            .expect("integer operation domains only accept integer-valued expressions");
        let right_value = integer_value_from_payload(rhs)
            .expect("integer operation domains only accept integer-valued expressions");
        let Some(range) = domain.integer_range() else {
            return Err(CompilerError::compiler_error(
                "Integer folding received a non-integer result domain.",
            )
            .into());
        };
        let value =
            checked_int_binary_result(left_value, right_value, range, numeric_operator, ctx)?;
        return Ok(integer_result_payload(domain, value));
    }

    let Some(precision) = domain.binary_float_precision() else {
        return Err(CompilerError::compiler_error(
            "Numeric folding received a domain without integer or binary-float arithmetic.",
        )
        .into());
    };
    let left_value = float_operand_in_domain(lhs, numeric.left_conversion)
        .expect("binary-float operation domains only accept numeric expressions");
    let right_value = float_operand_in_domain(rhs, numeric.right_conversion)
        .expect("binary-float operation domains only accept numeric expressions");

    let value = match numeric_operator {
        NumericOperator::Add => left_value + right_value,
        NumericOperator::Subtract => left_value - right_value,
        NumericOperator::Multiply => left_value * right_value,
        NumericOperator::Divide => {
            if right_value == 0.0 {
                return divide_by_zero_error(ctx);
            }
            left_value / right_value
        }
        NumericOperator::Remainder => {
            if right_value == 0.0 {
                return divide_by_zero_error(ctx);
            }
            left_value % right_value
        }
        NumericOperator::Power => numeric_power::pow(left_value, right_value),
        NumericOperator::IntegerDivide | NumericOperator::Negate => {
            return Err(CompilerError::compiler_error(format!(
                "Binary-float folding received unsupported numeric operator '{}'",
                ctx.operator.to_str()
            ))
            .into());
        }
    };
    let value = checked_float_result(value, precision, ctx)?;
    Ok(float_result_payload(domain, value))
}

fn comparison_result(op: &Operator, ordering: std::cmp::Ordering) -> Option<bool> {
    match op {
        Operator::Equality => Some(ordering.is_eq()),
        Operator::NotEqual => Some(!ordering.is_eq()),
        Operator::GreaterThan => Some(ordering.is_gt()),
        Operator::GreaterThanOrEqual => Some(!ordering.is_lt()),
        Operator::LessThan => Some(ordering.is_lt()),
        Operator::LessThanOrEqual => Some(!ordering.is_gt()),
        _ => None,
    }
}

fn fold_numeric_comparison(
    lhs: &ConstValuePayload,
    rhs: &ConstValuePayload,
    op: &Operator,
    comparison: &ResolvedComparisonOperation,
) -> Option<bool> {
    let ordering = match comparison.domain {
        NumericComparisonDomain::ExactInteger => {
            // Both values widen exactly into i128, including the complete U64 range.
            integer_value_from_payload(lhs)?.cmp(&integer_value_from_payload(rhs)?)
        }
        NumericComparisonDomain::BinaryFloat => {
            float_value_from_payload(lhs)?.partial_cmp(&float_value_from_payload(rhs)?)?
        }
        NumericComparisonDomain::Number(scale) => {
            let left = number_operand_in_domain(lhs, scale, comparison.left_conversion)?;
            let right = number_operand_in_domain(rhs, scale, comparison.right_conversion)?;
            left.coefficient().cmp(right.coefficient())
        }
    };
    comparison_result(op, ordering)
}

#[cfg(test)]
#[path = "tests/constant_folding_tests.rs"]
mod tests;
