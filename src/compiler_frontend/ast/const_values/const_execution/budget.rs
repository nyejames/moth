//! Bounded work, allocation and text-growth accounting for const execution.
//!
//! WHAT: owns the per-attempt execution budget and its aggregate module meter.
//! WHY:  the executor, required-call rewriting and bounded template folding must consume the same
//!       allowance without duplicating limit constants or accounting state. Interpreter control
//!       and recovery state lives in `state`, so this type answers only "how much work may this
//!       attempt still spend".

use super::values::{hard_limit_error, limit_error};
use super::{
    ConstExecutionMeter, ConstStringValue, ConstValueId, ConstValuePayload, ConstValueStore,
    FxHashSet, PathId, Rc, SourceSpan, TemplateOutputLimits, TemplateOutputMeter,
    TemplateWorkBudget,
};
use crate::compiler_frontend::ast::const_eval::{
    ConstBudgetCategory, ConstBudgetUnit, ConstantFoldError,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstValue, ConstValueField,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::cell::{Cell, RefCell};

/// Hard structural bound that configured work limits do not raise.
pub(super) const MAX_LOOP_ITERATIONS: usize = 1_000_000;
pub(super) const MAX_CALL_DEPTH: usize = 64;
pub(super) const MAX_EXPRESSION_DEPTH: usize = 256;
pub(super) const MAX_HANDLER_DEPTH: usize = 64;
pub(super) const MAX_PROJECTION_DEPTH: usize = 256;
pub(super) const MAX_RETAINED_VALUES: usize = 1_000_000;
pub(super) const MAX_TOTAL_RETAINED_VALUES: usize = MAX_RETAINED_VALUES * 8;

pub(super) struct ExecutionBudget {
    /// Shared aggregate accounting owned by the module const-value store.
    pub(super) meter: Rc<RefCell<ConstExecutionMeter>>,
    /// Last operation reached, so a budget refusal still points at the source that spent it.
    pub(super) span: Option<SourceSpan>,
    /// Aggregate values copied into live frames and projected writes in this attempt.
    pub(super) retained_values: usize,
    /// Aggregate UTF-8 bytes retained by folded text in this attempt.
    pub(super) retained_memory_bytes: usize,
    /// Work steps consumed by this attempt, shared with bounded template folds.
    pub(super) steps: Rc<Cell<usize>>,
    /// Contiguous const-value index range charged in this attempt.
    pub(super) charged_values: Option<(usize, usize)>,
    /// Rows whose allocation was charged before insertion by a copying mutation.
    pub(super) precharged_value_indices: FxHashSet<usize>,
    /// Active interpreted function calls with their call sites, outermost first.
    ///
    /// Call depth is a resource bound and the chain labels escaping diagnostics, so it stays with
    /// the budget rather than with control-flow state. A frame keeps only compact body paths;
    /// owned donor handles are resolved when a diagnostic escapes.
    pub(super) call_stack: Vec<ConstCallFrame>,
}

/// One active interpreted call: where it is written and which body contains that site.
///
/// WHY: the interpreter resolves the owning body's donor handle from the executables table only
///      when a call chain escapes. Storing the compact path here avoids cloning an Arc-backed
///      handle on every successful call.
pub(super) struct ConstCallFrame {
    /// Authored call site in the caller's source.
    pub(super) span: Option<SourceSpan>,
    /// Body that contains `span`; `None` means consumer-local source.
    pub(super) owner_body: Option<PathId>,
}

/// A call refused by the hard call-depth bound.
///
/// WHY: the budget owns the limit, but only the interpreter can resolve a body's donor identity
///      into the diagnostic's owned handles.
#[derive(Debug)]
pub(super) struct CallDepthRefusal {
    pub(super) span: Option<SourceSpan>,
    pub(super) owner_body: Option<PathId>,
}

const _: () = assert!(
    std::mem::size_of::<ConstCallFrame>() == 12,
    "the call stack is entered on every interpreted call; growing its frame needs a measured reason"
);

impl ExecutionBudget {
    pub(super) fn new(meter: Rc<RefCell<ConstExecutionMeter>>) -> Self {
        Self {
            meter,
            span: None,
            retained_values: 0,
            retained_memory_bytes: 0,
            steps: Rc::new(Cell::new(0)),
            charged_values: None,
            precharged_value_indices: FxHashSet::default(),
            call_stack: Vec::new(),
        }
    }

    pub(super) fn meter(&self) -> &Rc<RefCell<ConstExecutionMeter>> {
        &self.meter
    }

    pub(super) fn steps_cell(&self) -> &Rc<Cell<usize>> {
        &self.steps
    }

    pub(super) fn step(&mut self) -> Result<(), ConstantFoldError> {
        let span = self.span;
        self.meter
            .borrow_mut()
            .charge_step(&self.steps, span)
            .map_err(ConstantFoldError::Budget)
    }

    pub(super) fn retain_values(
        &mut self,
        count: usize,
        span: Option<SourceSpan>,
    ) -> Result<(), ConstantFoldError> {
        self.retained_values = self.retained_values.saturating_add(count);
        let mut meter = self.meter.borrow_mut();
        meter.retained_values = meter.retained_values.saturating_add(count);
        if self.retained_values > MAX_RETAINED_VALUES {
            return Err(hard_limit_error(
                span,
                ConstBudgetCategory::Memory,
                ConstBudgetUnit::RetainedValues,
                MAX_RETAINED_VALUES,
            ));
        }
        if meter.retained_values > MAX_TOTAL_RETAINED_VALUES {
            return Err(limit_error(
                span,
                ConstBudgetCategory::Memory,
                ConstBudgetUnit::RetainedValues,
                MAX_TOTAL_RETAINED_VALUES as u64,
                None,
                true,
            ));
        }
        Ok(())
    }

    /// Remaining output ceilings for one bounded template fold.
    pub(super) fn template_output_limits(&self, span: Option<SourceSpan>) -> TemplateOutputLimits {
        let meter = self.meter.borrow();
        let attempt_bytes = meter
            .limits
            .max_attempt_memory_bytes
            .saturating_sub(self.retained_memory_bytes);
        let total_bytes = meter
            .limits
            .max_total_memory_bytes
            .saturating_sub(meter.retained_memory_bytes);
        let byte_aggregate = total_bytes < attempt_bytes;
        let attempt_pieces = MAX_RETAINED_VALUES.saturating_sub(self.retained_values);
        let total_pieces = MAX_TOTAL_RETAINED_VALUES.saturating_sub(meter.retained_values);
        let piece_aggregate = total_pieces < attempt_pieces;
        TemplateOutputLimits {
            max_bytes: attempt_bytes.min(total_bytes),
            byte_limit: if byte_aggregate {
                meter.limits.max_total_memory_bytes
            } else {
                meter.limits.max_attempt_memory_bytes
            },
            byte_aggregate,
            max_structural_pieces: attempt_pieces.min(total_pieces),
            piece_limit: if piece_aggregate {
                MAX_TOTAL_RETAINED_VALUES
            } else {
                MAX_RETAINED_VALUES
            },
            piece_aggregate,
            span,
            meter: Rc::new(TemplateOutputMeter::default()),
        }
    }

    /// Share this attempt's step meter with one bounded template fold.
    pub(super) fn template_work(&self, span: Option<SourceSpan>) -> TemplateWorkBudget {
        TemplateWorkBudget {
            meter: Rc::clone(self.meter()),
            attempt_steps: Rc::clone(self.steps_cell()),
            span,
        }
    }

    pub(super) fn retain_memory_bytes(
        &mut self,
        bytes: usize,
        span: Option<SourceSpan>,
    ) -> Result<(), ConstantFoldError> {
        self.retained_memory_bytes = self.retained_memory_bytes.saturating_add(bytes);
        let mut meter = self.meter.borrow_mut();
        meter.retained_memory_bytes = meter.retained_memory_bytes.saturating_add(bytes);
        if self.retained_memory_bytes > meter.limits.max_attempt_memory_bytes {
            return Err(limit_error(
                span,
                ConstBudgetCategory::Memory,
                ConstBudgetUnit::Mib,
                meter.limits.max_attempt_memory_bytes as u64,
                Some("const_memory_limit_mb"),
                false,
            ));
        }
        if meter.retained_memory_bytes > meter.limits.max_total_memory_bytes {
            return Err(limit_error(
                span,
                ConstBudgetCategory::Memory,
                ConstBudgetUnit::Mib,
                meter.limits.max_total_memory_bytes as u64,
                Some("const_memory_limit_mb"),
                true,
            ));
        }
        Ok(())
    }

    pub(super) fn enter_call(
        &mut self,
        span: Option<SourceSpan>,
        owner_body: Option<PathId>,
    ) -> Result<(), CallDepthRefusal> {
        if self.call_stack.len() >= MAX_CALL_DEPTH {
            return Err(CallDepthRefusal { span, owner_body });
        }
        self.call_stack.push(ConstCallFrame { span, owner_body });
        Ok(())
    }

    pub(super) fn leave_call(&mut self) {
        self.call_stack.pop();
    }
}

/// Charge rows appended since `start`, keeping one contiguous covered range per attempt.
pub(super) fn charge_new_value_text(
    constants: &ConstValueStore,
    string_table: &StringTable,
    start: usize,
    budget: &mut ExecutionBudget,
) -> Result<(), ConstantFoldError> {
    let end = constants.value_count();
    let (mut from, mut to) = budget.charged_values.unwrap_or((start, start));
    if start < from {
        charge_value_rows(constants, string_table, start, from, budget)?;
        from = start;
    }
    if start > to {
        charge_value_rows(constants, string_table, to, start, budget)?;
        to = start;
    }
    if end > to {
        charge_value_rows(constants, string_table, to, end, budget)?;
        to = end;
    }
    budget.charged_values = Some((from, to));
    Ok(())
}

pub(super) fn charge_value_rows(
    constants: &ConstValueStore,
    string_table: &StringTable,
    start: usize,
    end: usize,
    budget: &mut ExecutionBudget,
) -> Result<(), ConstantFoldError> {
    for index in start..end {
        if budget.precharged_value_indices.contains(&index) {
            continue;
        }
        let Some(value) = constants.value_id_at(index) else {
            continue;
        };
        let (bytes, pieces) = value_own_allocation(constants, string_table, value);
        let span = constants.metadata(value).and_then(|m| m.span);
        budget.retain_memory_bytes(bytes, span)?;
        budget.retain_values(pieces, span)?;
    }
    Ok(())
}

/// Charges only the payload owned by one newly inserted value.
///
/// WHAT: string bytes, structural string pieces, one row slot and owned aggregate vector slots.
/// WHY: reachable child rows were already charged when their own indices were created. Recursing
///      from every new root would charge a shared child once per parent and make the budget depend
///      on graph shape instead of produced work.
pub(super) fn value_own_allocation(
    constants: &ConstValueStore,
    string_table: &StringTable,
    value: ConstValueId,
) -> (usize, usize) {
    let Some(payload) = constants.payload(value) else {
        return (0, 0);
    };
    let row_bytes = std::mem::size_of::<ConstValue>();
    let (owned_bytes, string_pieces) = match payload {
        ConstValuePayload::String(string) => (
            string_bytes(string, string_table)
                .saturating_add(string_pieces(string) * std::mem::size_of::<ConstStringPiece>()),
            string_pieces(string),
        ),
        ConstValuePayload::Template { folded, .. } => folded.as_ref().map_or((0, 0), |string| {
            (
                string_bytes(string, string_table).saturating_add(
                    string_pieces(string) * std::mem::size_of::<ConstStringPiece>(),
                ),
                string_pieces(string),
            )
        }),
        ConstValuePayload::Collection(items) => (
            items.len() * std::mem::size_of::<ConstValueId>(),
            items.len(),
        ),
        ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => (
            fields.len() * std::mem::size_of::<ConstValueField>(),
            fields.len(),
        ),
        ConstValuePayload::Range { .. } => (2 * std::mem::size_of::<ConstValueId>(), 2),
        ConstValuePayload::Coerced(_) | ConstValuePayload::OptionSome(_) => {
            (std::mem::size_of::<ConstValueId>(), 1)
        }
        ConstValuePayload::Number(_)
        | ConstValuePayload::FixedScalar(_)
        | ConstValuePayload::Bool(_)
        | ConstValuePayload::Char(_)
        | ConstValuePayload::OptionNone => (0, 0),
    };
    (
        row_bytes.saturating_add(owned_bytes),
        string_pieces.saturating_add(1),
    )
}

pub(super) fn string_pieces(string: &ConstStringValue) -> usize {
    match string {
        ConstStringValue::Text(_) => 0,
        ConstStringValue::Pieces(pieces) => pieces.len(),
    }
}

pub(super) fn string_bytes(string: &ConstStringValue, string_table: &StringTable) -> usize {
    match string {
        ConstStringValue::Text(text) => string_table.resolve(*text).len(),
        ConstStringValue::Pieces(pieces) => pieces
            .iter()
            .map(|piece| match piece {
                ConstStringPiece::Text(text) => string_table.resolve(*text).len(),
                ConstStringPiece::Resource(_) | ConstStringPiece::SiteRoot => 0,
            })
            .sum(),
    }
}
