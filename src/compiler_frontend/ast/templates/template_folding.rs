//! Compile-time template folding over immutable, owner-bound semantic expressions.
//!
//! Exact prepared TIR views retain structural authority. Bindings share committed IDs;
//! the ordinary expression folder owns isolated replacement construction and fold refusal.

use crate::compiler_frontend::ast::const_eval::{
    BindingFold, BindingFoldResult, ConstantFoldError, ConstantFoldOutcome,
    ConstantFoldRefusalReason, FoldRequirement, FoldServices, constant_fold,
    fold_expression_with_bindings,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstExecutionMeter, ConstStringValue, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView,
};
use crate::compiler_frontend::ast::module_ast::scope_context::ScopeContext;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template_control_flow::TemplateFoldBinding;
use crate::compiler_frontend::ast::templates::tir::{
    FoldedConstTemplatePiece, TemplateIrStore, TemplatePreparationMode, TemplateTirPhase,
    TemplateWrapperReference, TirView, WrapperSlotRouting, prepare_tir_view,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::invalid_template_structure;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

use rustc_hash::FxHashMap;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Executes required const calls in one TIR expression site at the moment the fold selects it.
///
/// WHAT: gives the const executor a callback at the exact selected expression site.
/// WHY: a template call inside a branch or loop body can only run with that site's active
///      bindings; pre-rewriting every collected site would execute inactive branches and fail
///      loop-body references before their bindings exist.
pub(crate) trait RequiredCallSiteRewriter {
    fn rewrite_site(
        &mut self,
        expression: ExprId,
        expression_builder: &mut AstExpressionBuilder,
        const_values: &mut ConstValueStore,
        string_table: &mut StringTable,
        source_scope: Option<&ScopeContext>,
        bindings: &[TemplateFoldBinding],
    ) -> Result<ExprId, ConstantFoldError>;
}

/// Byte and structural-piece ceilings supplied by a bounded consumer, with the charges already
/// accumulated against them.
///
/// WHY: a template loop can multiply one retained string far beyond the instruction budget. The
/// reducer checks these before reserving or appending output, so a bounded caller fails at the
/// owning budget instead of exhausting memory.
#[derive(Clone, Debug)]
pub(crate) struct TemplateOutputLimits {
    /// Remaining byte allowance for this fold.
    pub(crate) max_bytes: usize,
    /// Full byte ceiling that controls the remaining allowance.
    pub(crate) byte_limit: usize,
    pub(crate) byte_aggregate: bool,
    /// Remaining structural-piece allowance for this fold.
    pub(crate) max_structural_pieces: usize,
    /// Full structural-piece ceiling that controls the remaining allowance.
    pub(crate) piece_limit: usize,
    pub(crate) piece_aggregate: bool,
    pub(crate) span: Option<SourceSpan>,
    /// Charges accumulated by every reducer output state below this consumer.
    ///
    /// WHY: a loop folds each iteration into a fresh output state and wrappers add levels of
    /// their own. Per-state counters would restart at zero for each of them and let their
    /// combined output pass the consumer's ceiling.
    pub(crate) meter: Rc<TemplateOutputMeter>,
}

/// Running output totals for one bounded template fold.
///
/// WHAT: counts the bytes and structural pieces the reducer materialises for one consumer.
/// WHY: the charge must be cumulative across every iteration and nested level of that fold, while
/// each retained copy is still charged exactly once: the state that first materialises it pays,
/// and an enclosing state that concatenates the finished emission does not.
#[derive(Debug, Default)]
pub(crate) struct TemplateOutputMeter {
    pub(crate) retained_bytes: Cell<usize>,
    pub(crate) retained_pieces: Cell<usize>,
}

impl TemplateOutputMeter {
    /// Adds the bytes of one materialised copy and returns the fold's running total.
    pub(crate) fn charge_bytes(&self, bytes: usize) -> usize {
        let total = self.retained_bytes.get().saturating_add(bytes);
        self.retained_bytes.set(total);
        total
    }

    /// Adds one retained structural piece and returns the fold's running total.
    pub(crate) fn charge_piece(&self) -> usize {
        let total = self.retained_pieces.get().saturating_add(1);
        self.retained_pieces.set(total);
        total
    }
}

/// Shared work meter for one bounded const-execution template fold.
///
/// WHY: template loops can execute many low-output iterations. The reducer charges each iteration
/// to the same aggregate execution meter required calls use, so total work stays bounded even
/// when the output ceiling is never reached. Attempt and aggregate limits are read from that
/// shared meter rather than copied here.
#[derive(Clone, Debug)]
pub(crate) struct TemplateWorkBudget {
    pub(crate) meter: Rc<RefCell<ConstExecutionMeter>>,
    pub(crate) attempt_steps: Rc<Cell<usize>>,
    pub(crate) span: Option<SourceSpan>,
}

pub(crate) struct TirFoldContext<'a> {
    pub string_table: &'a mut StringTable,
    pub expression_builder: &'a mut AstExpressionBuilder,
    pub const_values: &'a mut ConstValueStore,
    pub template_const_loop_iteration_limit: usize,
    /// Optional output ceiling for a bounded const-execution attempt.
    pub(crate) template_output_limits: Option<TemplateOutputLimits>,
    /// Optional work meter for a bounded const-execution template fold.
    pub(crate) template_work: Option<TemplateWorkBudget>,
    pub(crate) bindings: Vec<TemplateFoldBinding>,
    pub(crate) source_scope: Option<&'a ScopeContext>,
    pub(crate) external_package_registry: Option<&'a ExternalPackageRegistry>,
    /// Slot routing prepared for wrapper edges during this fold.
    ///
    /// A wrapper reference's slot schema depends only on the immutable TIR store and the
    /// reference's exact context. Every direct child that inherits the same `$before`/`$after`
    /// composite therefore reuses one entry instead of repeating the schema walk. Entries are
    /// keyed by the exact reference, so reuse is a hash lookup rather than a scan over every
    /// prepared edge. The map is fold-local preparation, not a cross-compilation result cache:
    /// no entry outlives this fold, every fold context reads one store for its whole lifetime
    /// and templates without inherited wrappers allocate none.
    pub(crate) prepared_wrapper_slot_routing:
        FxHashMap<TemplateWrapperReference, WrapperSlotRouting>,
    /// Optional executor callback for selected required-call expression sites.
    pub(crate) required_call_rewriter: Option<&'a mut dyn RequiredCallSiteRewriter>,
}

/// Structural no-output is distinct from an emitted empty string for parent wrappers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TemplateEmission {
    NoOutput,
    Output(ConstStringValue),
}

/// Provenance follows only the exact selected emission path, never an ambient accumulator.
#[derive(Debug)]
pub(crate) struct TemplateFoldResult {
    pub(crate) emission: TemplateEmission,
    pub(crate) provenance: SyntheticInterfaceProvenance,
    pub(crate) projection_pieces: Option<Vec<FoldedConstTemplatePiece>>,
    /// Set when an operation inside the fold refused to produce text, so `emission` is empty.
    ///
    /// The consuming boundary owns the decision: a const-required context reports the refusal as
    /// its diagnostic, while a value context keeps the template for runtime materialization.
    pub(crate) refusal: Option<ConstantFoldRefusalReason>,
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
            refusal: None,
        }
    }

    pub(crate) fn with_refusal(mut self, refusal: Option<ConstantFoldRefusalReason>) -> Self {
        self.refusal = refusal;
        self
    }

    /// Converts a recorded refusal into the diagnostic a required-text boundary must report.
    ///
    /// WHAT: consumes the refusal and leaves the emission, provenance and projection pieces alone.
    /// WHY: every required-text boundary reports the same refusal diagnostic, while opportunistic
    ///      boundaries keep the result to materialize the template at runtime.
    pub(crate) fn take_required_diagnostic(&mut self) -> Option<DiagnosticDraft> {
        self.refusal
            .take()
            .map(ConstantFoldRefusalReason::into_draft)
    }
}
impl<'a> TirFoldContext<'a> {
    /// Creates a fold context with an optional module registry but no active bindings or source scope.
    pub(crate) fn new(
        string_table: &'a mut StringTable,
        expression_builder: &'a mut AstExpressionBuilder,
        const_values: &'a mut ConstValueStore,
        external_package_registry: Option<&'a ExternalPackageRegistry>,
        template_const_loop_iteration_limit: usize,
    ) -> Self {
        Self {
            string_table,
            expression_builder,
            const_values,
            external_package_registry,
            template_const_loop_iteration_limit,
            template_output_limits: None,
            template_work: None,
            bindings: Vec::new(),
            source_scope: None,
            prepared_wrapper_slot_routing: FxHashMap::default(),
            required_call_rewriter: None,
        }
    }
}

impl TirFoldContext<'_> {
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

pub(crate) fn resolve_fold_bindings_in_expression(
    mut expression: ExprId,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<BindingFoldResult, TemplateError> {
    if let Some(rewriter) = fold_context.required_call_rewriter.as_deref_mut() {
        expression = rewriter.rewrite_site(
            expression,
            fold_context.expression_builder,
            fold_context.const_values,
            fold_context.string_table,
            fold_context.source_scope,
            &fold_context.bindings,
        )?;
    }
    let original = fold_context.expression_builder.view(expression)?;
    if let ExpressionOperationView::Reference(path) = original.operation()
        && let Some(binding) = fold_context
            .bindings
            .iter()
            .rev()
            .find(|binding| binding.path == path)
    {
        // A binding is already a completed value. Preserve optional wrappers and nested
        // template identity rather than evaluating or reconstructing the bound root.
        fold_context.expression_builder.view(binding.value)?;
        return Ok(BindingFoldResult {
            expression: binding.value,
            refusal: None,
        });
    }
    let bindings = &fold_context.bindings;
    let resolved = if bindings.is_empty() {
        BindingFoldResult {
            expression,
            refusal: None,
        }
    } else {
        fold_expression_with_bindings(
            expression,
            fold_context.expression_builder,
            fold_context.const_values,
            fold_context.string_table,
            BindingFold::speculative(
                fold_context.source_scope,
                fold_context.external_package_registry,
            ),
            |path| {
                bindings
                    .iter()
                    .rev()
                    .find(|binding| binding.path == path)
                    .map(|binding| binding.value)
            },
        )?
    };
    // Rendering a coerced template enters that nested value's own exact context.
    let view = fold_context.expression_builder.view(resolved.expression)?;
    if resolved.expression == expression
        && let ExpressionOperationView::Coerce { value, .. } = view.operation()
        && matches!(value.operation(), ExpressionOperationView::Template(_))
    {
        return Ok(BindingFoldResult {
            expression: value.id(),
            refusal: resolved.refusal,
        });
    }
    Ok(resolved)
}

pub(crate) fn selected_option_capture_payload_with_provenance(
    scrutinee: ExprId,
    pattern: &MatchPattern,
    store: &TemplateIrStore,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<(Option<TemplateFoldBinding>, SyntheticInterfaceProvenance), TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(scrutinee, fold_context)?.expression;
    let view = fold_context.expression_builder.view(resolved)?;
    let provenance = expression_provenance(view);
    let payload = match view.operation() {
        ExpressionOperationView::Constant(id) => match fold_context.const_values.payload(id) {
            Some(ConstValuePayload::OptionNone) => return Ok((None, provenance)),
            Some(ConstValuePayload::OptionSome(value) | ConstValuePayload::Coerced(value)) => {
                let metadata = fold_context.const_values.metadata(*value).ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "Missing option capture constant.",
                    )
                })?;
                let value = *value;
                let type_id = metadata.type_id;
                let span = view.span();
                let mode = view.value_mode();
                fold_context
                    .expression_builder
                    .constant(value, type_id, span, mode)?
                    .id()
            }
            _ => return Err(option_capture_const_deferred_error(view).into()),
        },
        ExpressionOperationView::Coerce { value, .. } => {
            let constant = value
                .const_value_kind_with_template_classifier(
                    fold_context.const_values,
                    &mut |template| {
                        let reference = template.tir_reference;
                        let view = TirView::with_minimum_phase(
                            store,
                            reference.root,
                            reference.phase,
                            TemplateTirPhase::Composed,
                            reference.context,
                        )?;
                        Ok(prepare_tir_view(
                            &view,
                            fold_context.expression_builder,
                            fold_context.const_values,
                            TemplatePreparationMode::Value,
                            fold_context.external_package_registry,
                        )?
                        .facts
                        .final_value_kind)
                    },
                )?
                .is_compile_time_value();
            if !constant {
                return Err(option_capture_const_deferred_error(view).into());
            }
            value.id()
        }
        _ => return Err(option_capture_const_deferred_error(view).into()),
    };
    let MatchPattern::OptionPresentCapture { binding_path, .. } = pattern else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            "Template option-capture folding received a non-capture pattern.",
        )
        .into());
    };
    let provenance = provenance.union(&expression_provenance(
        fold_context.expression_builder.view(payload)?,
    ));
    Ok((
        Some(TemplateFoldBinding {
            path: *binding_path,
            value: payload,
        }),
        provenance,
    ))
}

fn expression_provenance(expression: ExpressionView<'_>) -> SyntheticInterfaceProvenance {
    expression
        .occurrence_facts()
        .map_or_else(SyntheticInterfaceProvenance::empty, |facts| {
            facts.provenance.clone()
        })
}

fn option_capture_const_deferred_error(expression: ExpressionView<'_>) -> DiagnosticDraft {
    invalid_template_structure(
        InvalidTemplateStructureReason::TemplateOptionCaptureConstDeferred,
        expression.span(),
    )
}

/// One folded TIR condition operand.
///
/// WHAT: either the constant `Bool` the condition settled to, or the refusal that kept it
///       dynamic. Both carry the condition's synthetic provenance for the consuming node.
pub(crate) enum FoldedCondition {
    Selected {
        value: bool,
        provenance: SyntheticInterfaceProvenance,
    },
    Refused {
        reason: ConstantFoldRefusalReason,
        provenance: SyntheticInterfaceProvenance,
    },
}

/// Folds one `if` or `loop` condition through the shared constant engine.
///
/// WHAT: resolves fold bindings, folds the condition opportunistically, then reads the constant
///       `Bool` it settled to.
/// WHY: one owner serves both conditional forms, so a registered Core call in a condition folds
///      like any other fold operand, and a structural-text refusal keeps the template dynamic
///      instead of reporting the generic "condition is not const" structure error.
pub(crate) fn fold_condition_with_provenance(
    condition: ExprId,
    fallback_span: Option<SourceSpan>,
    not_constant: InvalidTemplateStructureReason,
    fold_context: &mut TirFoldContext<'_>,
) -> Result<FoldedCondition, TemplateError> {
    let resolved = resolve_fold_bindings_in_expression(condition, fold_context)?;
    let provenance =
        expression_provenance(fold_context.expression_builder.view(resolved.expression)?);
    if let Some(reason) = resolved.refusal {
        return Ok(FoldedCondition::Refused { reason, provenance });
    }
    let folded = match constant_fold(
        resolved.expression,
        fold_context.expression_builder,
        fold_context.const_values,
        FoldServices::new(
            &mut *fold_context.string_table,
            fold_context.source_scope,
            fold_context.external_package_registry,
        ),
        FoldRequirement::Opportunistic,
    ) {
        Ok(
            ConstantFoldOutcome::Folded(expression) | ConstantFoldOutcome::NotConstant(expression),
        ) => expression,
        Ok(ConstantFoldOutcome::Refused { reason, .. }) => {
            return Ok(FoldedCondition::Refused { reason, provenance });
        }
        Err(
            ConstantFoldError::Diagnostic(diagnostic)
            | ConstantFoldError::NumericFailure(diagnostic),
        ) if diagnostic.code() != "MOTH-SYNTAX-0037" => resolved.expression,
        Err(error) => return Err(error.into()),
    };
    let view = fold_context.expression_builder.view(folded)?;
    let value = const_bool_value(view, fold_context.const_values)
        .ok_or_else(|| invalid_template_structure(not_constant, view.span().or(fallback_span)))?;
    Ok(FoldedCondition::Selected { value, provenance })
}

/// The `Bool` a constant condition settles to, read through transparent coercion wrappers.
fn const_bool_value(condition: ExpressionView<'_>, constants: &ConstValueStore) -> Option<bool> {
    match condition.operation() {
        ExpressionOperationView::Constant(id) => {
            let mut payload = constants.payload(id);
            while let Some(ConstValuePayload::Coerced(value)) = payload {
                payload = constants.payload(*value);
            }
            if let Some(ConstValuePayload::Bool(value)) = payload {
                return Some(*value);
            }
            None
        }
        ExpressionOperationView::Coerce { value, .. } => const_bool_value(value, constants),
        _ => None,
    }
}
