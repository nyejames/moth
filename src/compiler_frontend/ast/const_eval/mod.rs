//! Conservative constant folding over the phase-owned expression DAG.
//!
//! Operations execute their retained resolved domains against borrowed constant content.
//! Fold output stays producer-local until the complete root is accepted; refusal retains
//! the original root and never publishes an intermediate constant or expression row.
mod external_calls;

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
use crate::compiler_frontend::ast::field_access::project_explicit_const_record_field;
use crate::compiler_frontend::ast::statements::value_production::expression_build::commit_constant_result_block;
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::builtins::casts::targets::BuiltinCastPolicyId;
use crate::compiler_frontend::builtins::casts::{BuiltinCastLiteral, apply_builtin_cast_policy};
use crate::compiler_frontend::compiler_errors::{InfrastructureErrors, InfrastructureFailure};
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data;
use crate::compiler_frontend::compiler_messages::diagnostics::families::choices_control_flow;
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelDraftMessage;
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, InvalidCastReason,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::number::{NumberArithmeticError, NumberValue};
use crate::compiler_frontend::datatypes::numeric_operators::{
    NumericComparisonDomain, NumericOperator,
};
use crate::compiler_frontend::datatypes::numeric_power;
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::external_packages::{
    ExternalConstEvalOp, ExternalPackageRegistry, TextConstEvalOp,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use moth_lexical::numeric::decimal::NumberScale;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarClass, FixedScalarValue};
use moth_lexical::numeric::precision::BinaryFloatPrecision;
use std::borrow::Cow;

#[derive(Debug)]
pub(crate) enum ConstantFoldError {
    Diagnostic(DiagnosticDraft),
    NumericFailure(DiagnosticDraft),
    Infrastructure(Box<InfrastructureErrors>),
}
#[derive(Debug)]
pub(crate) enum ConstantFoldOutcome {
    Folded(ExprId),
    NotConstant(ExprId),
    Refused {
        root: ExprId,
        reason: ConstantFoldRefusalReason,
    },
}

#[derive(Debug)]
pub(crate) enum ConstantFoldRefusalReason {
    StructuralTextUnavailable(DiagnosticDraft),
    EvaluationOutputBoundExceeded(DiagnosticDraft),
}

impl ConstantFoldRefusalReason {
    pub(crate) fn into_draft(self) -> DiagnosticDraft {
        match self {
            Self::StructuralTextUnavailable(diagnostic)
            | Self::EvaluationOutputBoundExceeded(diagnostic) => diagnostic,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FoldRequirement {
    Required,
    Opportunistic,
}

/// Shared borrowed services supplied to every expression-folding entry point.
pub(crate) struct FoldServices<'a> {
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) scope: Option<&'a ScopeContext>,
    pub(crate) registry: Option<&'a ExternalPackageRegistry>,
}

impl<'a> FoldServices<'a> {
    pub(crate) fn new(
        string_table: &'a mut StringTable,
        scope: Option<&'a ScopeContext>,
        registry: Option<&'a ExternalPackageRegistry>,
    ) -> Self {
        Self {
            string_table,
            scope,
            registry,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstStringRequirement {
    EqualityComparison,
    CastOrParse,
    CompileTimeMapKey,
    DuplicateKeyValidation,
    ExternalCall,
}

impl ConstStringRequirement {
    fn operation_name(self) -> &'static str {
        match self {
            Self::EqualityComparison => "string equality comparison",
            Self::CastOrParse => "string cast or parse",
            Self::CompileTimeMapKey => "compile-time map key",
            Self::DuplicateKeyValidation => "duplicate map-key validation",
            Self::ExternalCall => "Core Text operation",
        }
    }
}

/// Returns the operation for a registered infallible host call. The operation's slot table
/// owns the result arity; materialisation checks it against the call's success types.
pub(crate) fn registered_external_const_eval_op(
    expression: ExpressionView<'_>,
    registry: Option<&ExternalPackageRegistry>,
) -> Option<ExternalConstEvalOp> {
    let ExpressionOperationView::Call(call) = expression.operation() else {
        return None;
    };
    if call.handling().is_some() || call.metadata().error_type.is_some() {
        return None;
    }
    let ResolvedCallableTarget::Host(function_id) = call.metadata().target else {
        return None;
    };
    registry?.get_function_by_id(function_id)?.const_eval
}

impl From<DiagnosticDraft> for ConstantFoldError {
    fn from(diagnostic: DiagnosticDraft) -> Self {
        Self::Diagnostic(diagnostic)
    }
}

impl From<InfrastructureFailure> for ConstantFoldError {
    fn from(error: InfrastructureFailure) -> Self {
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
            TemplateError::Diagnostic(diagnostic)
            | TemplateError::EmptyArguments(diagnostic)
            | TemplateError::NonConstantReferenceInConstant(diagnostic) => {
                Self::Diagnostic(diagnostic)
            }
            TemplateError::Infrastructure(error) => Self::Infrastructure(error),
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
) -> Result<Option<StringId>, DiagnosticDraft> {
    let ExpressionOperationView::Constant(id) = value.operation() else {
        return Ok(None);
    };
    let Some(payload) = constants.payload(id) else {
        return Ok(None);
    };
    concrete_payload_text(payload, value.span(), requirement, strings)
}
pub(super) fn concrete_payload_text(
    payload: &ConstValuePayload,
    span: Option<SourceSpan>,
    requirement: ConstStringRequirement,
    strings: &mut StringTable,
) -> Result<Option<StringId>, DiagnosticDraft> {
    let ConstValuePayload::String(value) = payload else {
        return Ok(None);
    };
    match value {
        ConstStringValue::Text(text) => Ok(Some(*text)),
        ConstStringValue::Pieces(pieces) => {
            let mut text = String::new();
            for piece in pieces {
                match piece {
                    ConstStringPiece::Text(part) => text.push_str(strings.resolve(*part)),
                    ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => {
                        let operation = strings.intern(requirement.operation_name());
                        return Err(choices_control_flow::compile_time_evaluation_error(
                            CompileTimeEvaluationErrorReason::StructuralStringRequiresFinalText,
                            Some(operation),
                            span,
                        ));
                    }
                }
            }
            Ok(Some(strings.get_or_intern(text)))
        }
    }
}

use crate::compiler_frontend::ast::expressions::expression_rewrite::rewrite_expression_children;
/// Whether the consumer needs a complete constant or may retain a runtime expression.
#[derive(Clone, Copy)]
enum FoldBoundary<'a> {
    Runtime {
        requirement: FoldRequirement,
    },
    Content {
        types: &'a TypeEnvironment,
        requirement: FoldRequirement,
    },
}

impl<'a> FoldBoundary<'a> {
    fn requirement(self) -> FoldRequirement {
        match self {
            Self::Runtime { requirement } | Self::Content { requirement, .. } => requirement,
        }
    }

    fn folds_composites(self) -> bool {
        matches!(self, Self::Content { .. })
    }

    /// The type environment that composite folds consult; only constant content boundaries carry one.
    fn types(self) -> Result<&'a TypeEnvironment, InfrastructureFailure> {
        match self {
            Self::Content { types, .. } => Ok(types),
            Self::Runtime { .. } => crate::compiler_bug!(
                CompilerStage::ConstantEvaluation,
                "Composite constant folding requires a constant content boundary.",
            ),
        }
    }
}

/// Buffers one fold reuses for every trusted call it settles, so a call allocates no argument,
/// payload or slot vector of its own.
#[derive(Default)]
struct ExternalCallScratch {
    arguments: Vec<Option<ConstValueId>>,
    payloads: Vec<Option<ConstValuePayload>>,
    slots: Vec<ConstValueId>,
}

/// Executes against an immutable expression prefix and a discardable constant candidate.
struct FoldContext<'a, 'prefix, F> {
    values: &'a mut ConstValueCandidate<'prefix>,
    strings: &'a mut StringTable,
    lookup: F,
    boundary: FoldBoundary<'a>,
    scope: Option<&'a ScopeContext>,
    registry: Option<&'a ExternalPackageRegistry>,
    /// Expression rows the walk reads. The fold never mutates them.
    expressions: &'a AstExpressionBuilder,
    refusal: Option<ConstantFoldRefusalReason>,
    external_calls: ExternalCallScratch,
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
        inherited_provenance: Option<&SyntheticInterfaceProvenance>,
    ) -> Result<Option<ConstValueId>, ConstantFoldError> {
        let mut metadata = ConstValueStore::metadata_from_view(expression, kind)?;
        if let Some(provenance) = inherited_provenance {
            metadata.synthetic_interface_provenance.merge(provenance);
        }
        Ok(Some(self.values.insert_value(metadata, payload)?))
    }

    /// Labels a numeric failure with the #Config inputs its operand constants retained. Each numeric
    /// operator labels its own operands, as the baseline did.
    fn label_numeric_failure(
        &mut self,
        error: ConstantFoldError,
        operands: &[ConstValueId],
    ) -> ConstantFoldError {
        let ConstantFoldError::NumericFailure(diagnostic) = error else {
            return error;
        };
        let provenances = operands
            .iter()
            .filter_map(|operand| self.values.metadata(*operand))
            .map(|metadata| &metadata.synthetic_interface_provenance)
            .collect::<Vec<_>>();
        match attach_source_config_origin_labels(diagnostic, &provenances, self.strings, self.scope)
        {
            Ok(diagnostic) => ConstantFoldError::NumericFailure(diagnostic),
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
            None => Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Constant fold walk finished without a root result.",
            )
            .into()),
        }
    }

    /// Settles one trusted call whose argument results are known. A refusal records its reason and
    /// returns `false`. A folded call leaves its slot ids in `external_calls.slots`.
    fn settle_external_call(
        &mut self,
        operation: TextConstEvalOp,
        call: ExpressionView<'_>,
        arguments: &[Option<ConstValueId>],
    ) -> Result<bool, ConstantFoldError> {
        let evaluation = external_calls::evaluate_external_call(
            operation,
            arguments,
            self.values,
            self.strings,
            call.span(),
            &mut self.external_calls.payloads,
        )?;
        match evaluation {
            external_calls::ExternalCallEvaluation::Refused(reason) => {
                self.refusal.get_or_insert(reason);
                Ok(false)
            }
            external_calls::ExternalCallEvaluation::Folded => {
                let mut provenance = SyntheticInterfaceProvenance::empty();
                for argument in arguments.iter().flatten() {
                    let argument_metadata = self.values.metadata(*argument).ok_or_else(|| {
                        InfrastructureFailure::internal(
                            CompilerStage::ConstantEvaluation,
                            "Settled external-call argument lost its metadata.",
                        )
                    })?;
                    provenance.merge(&argument_metadata.synthetic_interface_provenance);
                }
                external_calls::materialise_success_slots(
                    operation.success_slot_types(),
                    call.success_types(),
                    self.external_calls.payloads.drain(..),
                    call,
                    &provenance,
                    self.values,
                    &mut self.external_calls.slots,
                )?;
                Ok(true)
            }
        }
    }

    /// Folds the arguments of a root Host call with zero or several result slots, then settles the
    /// call. Its slot ids are returned only when every argument and the operation settle.
    fn fold_host_result_slots(
        &mut self,
        operation: TextConstEvalOp,
        call: ExpressionView<'_>,
    ) -> Result<Option<Vec<ConstValueId>>, ConstantFoldError> {
        let ExpressionOperationView::Call(record) = call.operation() else {
            crate::compiler_bug!(
                CompilerStage::ConstantEvaluation,
                "Host result folding was given a row that is not a call.",
            );
        };
        // The argument buffer leaves `self` for the duration of the walks, because each walk
        // borrows `self` mutably.
        let mut arguments = std::mem::take(&mut self.external_calls.arguments);
        arguments.clear();
        for argument in record.arguments().iter() {
            arguments.push(self.fold_walk(argument.value().id())?);
        }
        let settled = arguments.iter().all(Option::is_some)
            && self.settle_external_call(operation, call, &arguments)?;
        self.external_calls.arguments = arguments;
        Ok(settled.then(|| std::mem::take(&mut self.external_calls.slots)))
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
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
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
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
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
            Operation::Call(call)
                if matches!(call.metadata().target, ResolvedCallableTarget::Host(_))
                    && call.handling().is_none()
                    && call.metadata().error_type.is_none() =>
            {
                walk.defer(
                    FoldStep::Complete {
                        expression: expression.id(),
                        base,
                    },
                    call.arguments()
                        .iter()
                        .map(|argument| argument.value().id()),
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
                if self.boundary.requirement() == FoldRequirement::Opportunistic {
                    return Ok(None);
                }
                let reason = match evidence {
                    ResolvedCastEvidence::Builtin { .. } => {
                        InvalidCastReason::BuiltinEvidenceNotConstFoldable
                    }
                    ResolvedCastEvidence::UserDefined { .. } => {
                        InvalidCastReason::UserDefinedEvidenceNotConstFoldable
                    }
                    ResolvedCastEvidence::GenericBound => {
                        InvalidCastReason::GenericBoundEvidenceNotConstFoldable
                    }
                };
                Err(calls_data::invalid_cast(
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
                        self.refusal.get_or_insert(
                            ConstantFoldRefusalReason::StructuralTextUnavailable(diagnostic),
                        );
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
            Operation::Call(call) => {
                let Some(ExternalConstEvalOp::Text(operation)) =
                    registered_external_const_eval_op(expression, self.registry)
                else {
                    return Ok(None);
                };
                if children.len() != call.arguments().len() {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "External call completion lost an argument result slot.",
                    )
                    .into());
                }
                if children.iter().any(Option::is_none) {
                    return Ok(None);
                }
                if !self.settle_external_call(operation, expression, children)? {
                    return Ok(None);
                }
                let &[value] = self.external_calls.slots.as_slice() else {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "Trusted Core Text call did not materialise its one success slot.",
                    )
                    .into());
                };
                return Ok(Some(value));
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
                    None,
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
                return self.insert(expression, payload, ConstValueKind::Composite, None);
            }
            Operation::Collection(_) if self.boundary.folds_composites() => {
                let Some(values) = children.iter().copied().collect::<Option<Vec<_>>>() else {
                    return Ok(None);
                };
                return self.insert(
                    expression,
                    ConstValuePayload::Collection(values),
                    ConstValueKind::Composite,
                    None,
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
                    None,
                );
            }
            Operation::Choice { tag, fields } if self.boundary.folds_composites() => {
                let types = self.boundary.types()?;
                let SuccessShape::One(type_id) = expression.success_shape() else {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "Folded choice lost its success type.",
                    )
                    .into());
                };
                let Some(fields) = record_fields(fields, children) else {
                    return Ok(None);
                };
                let nominal_path = *types.nominal_path(type_id).ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::ConstantEvaluation,
                        "Folded choice has no canonical nominal identity.",
                    )
                })?;
                let payload = ConstValuePayload::Choice {
                    nominal_path,
                    tag: tag as usize,
                    fields,
                };
                return self.insert(expression, payload, ConstValueKind::Composite, None);
            }
            _ => return Ok(None),
        };
        self.insert(expression, payload, ConstValueKind::Literal, None)
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
            crate::compiler_bug!(
                CompilerStage::ConstantEvaluation,
                "Cast completion lost its cast operands.",
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
                Err(diagnostic) => {
                    self.refusal.get_or_insert(
                        ConstantFoldRefusalReason::StructuralTextUnavailable(diagnostic),
                    );
                    return Ok(None);
                }
            },
            _ => return Ok(None),
        };
        let literal = apply_builtin_cast_policy(policy, &literal).map_err(|_| {
            calls_data::invalid_cast(
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
        ConstantFoldError::from(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
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
) -> Result<Option<(ExpressionView<'_>, &CastFacts)>, InfrastructureFailure> {
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
    TextUnavailable(DiagnosticDraft),
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
        return Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "Numeric unary operation lacks numeric facts.",
        )
        .into());
    };
    match numeric.domain {
        NumericScalar::Number(_) => {
            let ConstValuePayload::Number(value) = value else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "Dec negation received a non-Dec operand.",
                )
                .into());
            };
            Ok(ConstValuePayload::Number(value.negated()))
        }
        domain if domain.is_integer() => {
            let value = integer_value_from_payload(value).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "Integer negation received a non-integer operand.",
                )
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
            let precision = domain.binary_float_precision().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "Negation lacks a numeric domain.",
                )
            })?;
            let value = float_value_from_payload(value).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::ConstantEvaluation,
                    "Float negation received a non-float operand.",
                )
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

/// Payload families an explicit `#` constant may take when it replaces an operand.
///
/// WHY: numeric operators fold scalar payloads, while equality comparisons, template
/// interpolation, and trusted text calls read `String` payloads. One replacement rule serves every
/// consumer, so an explicit constant reaches the fold engine the same way whichever one reads it.
#[derive(Clone, Copy)]
pub(crate) enum ExplicitConstantPayload {
    Numeric,
    Text,
}

impl ExplicitConstantPayload {
    fn accepts(self, payload: &ConstValuePayload) -> bool {
        match self {
            Self::Numeric => matches!(
                payload,
                ConstValuePayload::FixedScalar(_) | ConstValuePayload::Number(_)
            ),
            Self::Text => matches!(payload, ConstValuePayload::String(_)),
        }
    }
}

/// Replace an operand that names an explicit `#` constant with that constant's value.
///
/// WHAT: a reference, a zero-projection copy, or a field of an explicit record constant becomes a
/// constant occurrence that keeps the operand's span and value mode. Returns `None` for an operand
/// without one result, and when no explicit constant of the operand's type and an accepted payload
/// backs the operand.
/// WHY: an explicit constant has one value wherever it is named, so operators, template
/// interpolation, and trusted host calls must read the same value.
pub(crate) fn explicit_constant_operand(
    operand: OwnedExpr,
    payloads: ExplicitConstantPayload,
    context: &ScopeContext,
    path_fork: &PathInternerFork,
    expressions: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
) -> Result<Option<OwnedExpr>, ExpressionConstructionFailure> {
    let view = expressions.view_owned(operand)?;
    // WHY: only a single success value can name an explicit constant. A template head may hold a
    // call with no or several success values; that keeps its own diagnostic instead of hitting
    // the single-value invariant of `value_type_id`.
    let Some(type_id) = view.success_shape().single() else {
        return Ok(None);
    };
    let span = view.span();
    let mode = view.value_mode();
    let replacement = match view.operation_id() {
        ExpressionOp::Reference(path) => context
            .explicit_constant_value(&path, path_fork, const_values)
            .filter(|&value| admits_constant(value, type_id, payloads, const_values))
            .map(|value| expressions.constant(value, type_id, span, mode))
            .transpose()?,
        ExpressionOp::Copy(place) if expressions.place(place)?.projections().len() == 0 => {
            let path = expressions.place(place)?.metadata().root;
            context
                .explicit_constant_value(&path, path_fork, const_values)
                .filter(|&value| admits_constant(value, type_id, payloads, const_values))
                .map(|value| expressions.constant(value, type_id, span, mode))
                .transpose()?
        }
        ExpressionOp::FieldAccess { .. } | ExpressionOp::Copy(_) => {
            project_explicit_const_record_field(
                operand,
                context,
                path_fork,
                expressions,
                const_values,
            )?
        }
        _ => None,
    };
    let Some(replacement) = replacement else {
        return Ok(None);
    };
    let replacement_view = expressions.view_owned(replacement)?;
    let admitted = match replacement_view.operation_id() {
        ExpressionOp::Constant(value) => {
            replacement_view.success_shape().single() == Some(type_id)
                && admits_constant(value, type_id, payloads, const_values)
        }
        _ => false,
    };
    if !admitted {
        return Ok(None);
    }
    Ok(Some(
        expressions.preserve_authored_metadata(replacement, operand)?,
    ))
}

fn admits_constant(
    value: ConstValueId,
    type_id: TypeId,
    payloads: ExplicitConstantPayload,
    const_values: &ConstValueStore,
) -> bool {
    let Some(metadata) = const_values.metadata(value) else {
        return false;
    };
    let Some(payload) = const_values.payload(value) else {
        return false;
    };
    metadata.type_id == type_id && payloads.accepts(payload)
}

pub(crate) fn constant_fold(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    services: FoldServices<'_>,
    requirement: FoldRequirement,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    #[cfg(feature = "benchmark_counters")]
    increment_frontend_counter(FrontendCounter::CensusConstantFoldEntries);
    let outcome = fold_root(
        root,
        builder,
        constants,
        services,
        FoldBoundary::Runtime { requirement },
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
        ConstantFoldOutcome::Refused { .. } => {
            increment_frontend_counter(FrontendCounter::CensusFoldOutcomeTextUnavailable)
        }
    }
    Ok(outcome)
}

/// Scope, registry and failure policy for one loop-binding substitution.
#[derive(Clone, Copy)]
pub(crate) struct BindingFold<'a> {
    scope: Option<&'a ScopeContext>,
    registry: Option<&'a ExternalPackageRegistry>,
    requirement: FoldRequirement,
    speculative: bool,
}

impl<'a> BindingFold<'a> {
    /// A rewritten operator whose fold fails with a diagnostic stays unfolded, so runtime
    /// lowering evaluates it. Baseline template folding treated such failures as speculative.
    pub(crate) fn speculative(
        scope: Option<&'a ScopeContext>,
        registry: Option<&'a ExternalPackageRegistry>,
    ) -> Self {
        Self {
            scope,
            registry,
            requirement: FoldRequirement::Opportunistic,
            speculative: true,
        }
    }

    /// A fold refusal is reported to the constant-required receiver.
    pub(crate) fn strict(
        scope: Option<&'a ScopeContext>,
        registry: Option<&'a ExternalPackageRegistry>,
    ) -> Self {
        Self {
            scope,
            registry,
            requirement: FoldRequirement::Required,
            speculative: false,
        }
    }
}

/// One binding substitution walk: the rewritten root and the refusal it recorded.
///
/// WHAT: a speculative walk keeps the first refusal beside the expression it left unfolded.
/// WHY: the caller reads that refusal directly instead of re-evaluating the same expression to
///      recover an outcome the walk already discarded. A strict walk reports its refusal as an
///      error, so its result never carries one.
#[derive(Debug)]
pub(crate) struct BindingFoldResult {
    pub(crate) expression: ExprId,
    pub(crate) refusal: Option<ConstantFoldRefusalReason>,
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
    /// The refusal a speculative fold reported, which the returned root stays unfolded for.
    refusal: Option<ConstantFoldRefusalReason>,
}

/// Substitute loop bindings without erasing coercion, template or runtime structure.
///
/// Rewrites bindings under `root` and speculatively folds rewritten constant parents. The walk is
/// iterative, so nesting depth grows the heap work stack rather than the Rust stack. Children are
/// visited in authored order and memoised when their substitution result settles.
pub(crate) fn fold_expression_with_bindings(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    strings: &mut StringTable,
    folding: BindingFold<'_>,
    mut lookup: impl FnMut(PathId) -> Option<ExprId>,
) -> Result<BindingFoldResult, ConstantFoldError> {
    let mut rewritten = rustc_hash::FxHashMap::default();
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
                let can_fold = match view.operation() {
                    ExpressionOperationView::Call(_) => {
                        registered_external_const_eval_op(view, folding.registry).is_some()
                    }
                    ExpressionOperationView::NumericUnary { .. }
                    | ExpressionOperationView::NumericBinary { .. }
                    | ExpressionOperationView::LogicalNot { .. }
                    | ExpressionOperationView::LogicalBinary { .. }
                    | ExpressionOperationView::Compare { .. }
                    | ExpressionOperationView::Range { .. } => true,
                    _ => false,
                };
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
                    // A refused speculative fold leaves the operator for runtime lowering, and the
                    // walk keeps the reason so its caller never folds the same row twice.
                    match constant_fold(
                        result,
                        builder,
                        constants,
                        FoldServices::new(&mut *strings, folding.scope, folding.registry),
                        folding.requirement,
                    ) {
                        Ok(ConstantFoldOutcome::Refused { reason, .. }) if !folding.speculative => {
                            return Err(reason.into_draft().into());
                        }
                        Ok(ConstantFoldOutcome::Refused { reason, root }) => {
                            walk.refusal.get_or_insert(reason);
                            root
                        }
                        Ok(outcome) => outcome_root(outcome),
                        // Capacity exhaustion is never a speculative refusal: it must reach the module build.
                        Err(ConstantFoldError::Diagnostic(diagnostic))
                            if folding.speculative && diagnostic.code() != "MOTH-SYNTAX-0037" =>
                        {
                            result
                        }
                        Err(ConstantFoldError::NumericFailure(diagnostic))
                            if folding.speculative && diagnostic.code() != "MOTH-SYNTAX-0037" =>
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
        Some((_, expression)) => Ok(BindingFoldResult {
            expression,
            refusal: walk.refusal,
        }),
        None => Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "Binding substitution finished without a root result.",
        )
        .into()),
    }
}

pub(crate) fn fold_expression_with_constants(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    services: FoldServices<'_>,
    types: &TypeEnvironment,
    requirement: FoldRequirement,
    lookup: impl FnMut(PathId) -> Option<ConstValueId>,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    fold_root(
        root,
        builder,
        constants,
        services,
        FoldBoundary::Content { types, requirement },
        lookup,
    )
}
/// Fold one constant-boundary expression (a declaration or signature default).
///
/// Unlike [`constant_fold`], this materialises every foldable constructor (collections, records, choices,
/// ranges and coercions) so the stored constant carries its full shape.
pub(crate) fn fold_constant_content(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    services: FoldServices<'_>,
    types: &TypeEnvironment,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    fold_root(
        root,
        builder,
        constants,
        services,
        FoldBoundary::Content {
            types,
            requirement: FoldRequirement::Required,
        },
        |_| None,
    )
}
fn outcome_root(outcome: ConstantFoldOutcome) -> ExprId {
    match outcome {
        ConstantFoldOutcome::Folded(root)
        | ConstantFoldOutcome::NotConstant(root)
        | ConstantFoldOutcome::Refused { root, .. } => root,
    }
}

/// Returns the trusted operation for a Runtime Host call with zero or several result slots.
/// One-slot calls keep the scalar path, which folds them to one `Constant`.
fn multi_result_host_op(
    boundary: FoldBoundary<'_>,
    expression: ExpressionView<'_>,
    registry: Option<&ExternalPackageRegistry>,
) -> Option<TextConstEvalOp> {
    if !matches!(boundary, FoldBoundary::Runtime { .. })
        || matches!(expression.success_shape(), SuccessShape::One(_))
    {
        return None;
    }
    match registered_external_const_eval_op(expression, registry)? {
        ExternalConstEvalOp::Text(operation) => Some(operation),
    }
}

fn fold_root(
    root: ExprId,
    builder: &mut AstExpressionBuilder,
    constants: &mut ConstValueStore,
    services: FoldServices<'_>,
    boundary: FoldBoundary<'_>,
    lookup: impl FnMut(PathId) -> Option<ConstValueId>,
) -> Result<ConstantFoldOutcome, ConstantFoldError> {
    let FoldServices {
        string_table: strings,
        scope,
        registry,
    } = services;
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
    let mut refusal = None;
    if let Some(operation) = multi_result_host_op(boundary, view, registry) {
        // A Host call with zero or several results folds into one constant result block. Its slot
        // rows commit as one unit; a refused or unsettled call leaves the authored call intact.
        let block_scope = scope.map(|context| context.scope);
        let slots = constants.stage_values(|values| {
            let mut context = FoldContext {
                values,
                strings,
                lookup,
                boundary,
                scope,
                registry,
                expressions: &*builder,
                refusal: None,
                external_calls: ExternalCallScratch::default(),
            };
            let slots = context.fold_host_result_slots(operation, view)?;
            refusal = context.refusal;
            Ok::<_, ConstantFoldError>(if refusal.is_some() || block_scope.is_none() {
                None
            } else {
                slots
            })
        })?;
        if let Some(reason) = refusal {
            return Ok(ConstantFoldOutcome::Refused { root, reason });
        }
        let (Some(slots), Some(block_scope)) = (slots, block_scope) else {
            return Ok(ConstantFoldOutcome::NotConstant(root));
        };
        return Ok(ConstantFoldOutcome::Folded(
            commit_constant_result_block(builder, root, &slots, block_scope, eligibility)?.id(),
        ));
    }
    let value = constants.stage_values(|values| {
        let mut context = FoldContext {
            values,
            strings,
            lookup,
            boundary,
            scope,
            registry,
            expressions: &*builder,
            refusal: None,
            external_calls: ExternalCallScratch::default(),
        };
        let value = context.fold_walk(view.id())?;
        refusal = context.refusal;
        Ok::<_, ConstantFoldError>(if refusal.is_some() { None } else { value })
    })?;
    if let Some(reason) = refusal {
        return Ok(ConstantFoldOutcome::Refused { root, reason });
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
    mut diagnostic: DiagnosticDraft,
    provenances: &[&SyntheticInterfaceProvenance],
    string_table: &mut StringTable,
    scope_context: Option<&ScopeContext>,
) -> Result<DiagnosticDraft, ConstantFoldError> {
    let mut input_names = provenances
        .iter()
        .flat_map(|provenance| provenance.source_config_member_names())
        .collect::<Vec<_>>();
    input_names.sort_unstable();
    input_names.dedup();
    if input_names.is_empty() {
        return Ok(diagnostic);
    }

    let Some(scope_context) = scope_context else {
        return Err(InfrastructureFailure::internal(CompilerStage::ConstantEvaluation,
            "A numeric compile-time diagnostic retained source #Config provenance without its owning scope resolver.",
        )
        .into());
    };

    for input_name in input_names {
        let Some((origin, Some(label_span))) =
            scope_context.source_config_diagnostic_origin(input_name)
        else {
            return Err(InfrastructureFailure::internal(CompilerStage::ConstantEvaluation, format!(
                "A numeric compile-time diagnostic retained source #Config input '{input_name}' without a matching resolver origin and source anchor."
            ))
            .into());
        };
        diagnostic = diagnostic.with_label(
            Some(label_span),
            Some(LabelDraftMessage::ConfigInputOrigin {
                input_name: string_table.intern(input_name),
                origin,
            }),
        );
    }

    Ok(diagnostic)
}

fn compile_time_evaluation_diagnostic(
    reason: CompileTimeEvaluationErrorReason,
    operation: Option<String>,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> ConstantFoldError {
    let operation = operation.map(|operation| string_table.get_or_intern(operation));
    ConstantFoldError::NumericFailure(choices_control_flow::compile_time_evaluation_error(
        reason, operation, span,
    ))
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
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                format!(
                    "Integer folding received unsupported numeric operator '{}'",
                    ctx.operator.to_str()
                ),
            )
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
                crate::compiler_bug!(
                    CompilerStage::ConstantEvaluation,
                    "integer result domain must be an integer scalar"
                )
            }
        },
        NumericScalar::Number(_) => crate::compiler_bug!(
            CompilerStage::ConstantEvaluation,
            "Number results use the exact Number constructor"
        ),
    }
}

fn float_result_payload(domain: NumericScalar, value: f64) -> ConstValuePayload {
    match domain {
        NumericScalar::Fixed(scalar) if scalar.class() == FixedScalarClass::BinaryFloat => {
            let value = FixedScalarValue::binary_float(scalar, value)
                .expect("rounded finite result fits the fixed float domain");
            ConstValuePayload::FixedScalar(value)
        }
        NumericScalar::Fixed(_) | NumericScalar::Number(_) => crate::compiler_bug!(
            CompilerStage::ConstantEvaluation,
            "float result domain must be a binary float scalar"
        ),
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
        return Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
            "Dec folding received a non-Dec operation domain.",
        )
        .into());
    };

    let result = if numeric_operator == NumericOperator::Power {
        let ConstValuePayload::Number(base) = lhs else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Dec power folding received a non-Dec base.",
            )
            .into());
        };
        // `Dec ^ I32` is the only Dec power row. The operator table already selected the
        // exact `I32` exponent, so any other exponent is a broken invariant rather than a
        // wider receiver repair.
        let ConstValuePayload::FixedScalar(exponent) = rhs else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Dec power folding received a non-I32 exponent.",
            )
            .into());
        };
        if exponent.scalar() != FixedScalar::I32 {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Dec power folding received a non-I32 exponent.",
            )
            .into());
        }
        let Some(exponent) = exponent.as_i64() else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Dec power folding received an unreadable I32 exponent.",
            )
            .into());
        };
        base.checked_power(exponent)
    } else {
        let Some(left_value) = number_operand_in_domain(lhs, scale, numeric.left_conversion) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Dec folding could not materialize the left operand in the result scale.",
            )
            .into());
        };
        let Some(right_value) = number_operand_in_domain(rhs, scale, numeric.right_conversion)
        else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
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
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                format!(
                    "Dec folding received unsupported numeric operator '{}'",
                    ctx.operator.to_str()
                ),
            )
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
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                "Integer folding received a non-integer result domain.",
            )
            .into());
        };
        let value =
            checked_int_binary_result(left_value, right_value, range, numeric_operator, ctx)?;
        return Ok(integer_result_payload(domain, value));
    }

    let Some(precision) = domain.binary_float_precision() else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::ConstantEvaluation,
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
            return Err(InfrastructureFailure::internal(
                CompilerStage::ConstantEvaluation,
                format!(
                    "Binary-float folding received unsupported numeric operator '{}'",
                    ctx.operator.to_str()
                ),
            )
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

#[cfg(test)]
#[path = "tests/folded_transport_tests.rs"]
mod folded_transport_tests;
