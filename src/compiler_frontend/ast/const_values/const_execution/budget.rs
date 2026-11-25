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
    PathId, Rc, SourceSpan, TemplateOutputLimits, TemplateOutputMeter, TemplateWorkBudget,
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
    /// Live cells, aggregate edges and folded value slots retained by this attempt.
    pub(super) retained_values: usize,
    /// Attempt memory, including scratch while it is held.
    pub(super) retained_memory_bytes: usize,
    /// Work steps consumed by this attempt, shared with bounded template folds.
    pub(super) steps: Rc<Cell<usize>>,
    /// Contiguous const-value index range charged in this attempt.
    pub(super) charged_values: Option<(usize, usize)>,
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
        let attempt_pieces = MAX_RETAINED_VALUES.saturating_sub(self.retained_values);
        let total_pieces = MAX_TOTAL_RETAINED_VALUES.saturating_sub(meter.retained_values);
        TemplateOutputLimits {
            max_bytes: attempt_bytes.min(total_bytes),
            byte_limit: if total_bytes < attempt_bytes {
                meter.limits.max_total_memory_bytes
            } else {
                meter.limits.max_attempt_memory_bytes
            },
            max_structural_pieces: attempt_pieces.min(total_pieces),
            piece_limit: if total_pieces < attempt_pieces {
                MAX_TOTAL_RETAINED_VALUES
            } else {
                MAX_RETAINED_VALUES
            },
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
            ));
        }
        if meter.retained_memory_bytes > meter.limits.max_total_memory_bytes {
            return Err(limit_error(
                span,
                ConstBudgetCategory::Memory,
                ConstBudgetUnit::Mib,
                meter.limits.max_total_memory_bytes as u64,
                Some("const_memory_limit_mb"),
            ));
        }
        Ok(())
    }

    pub(super) fn charge_scratch(
        &mut self,
        held: &mut usize,
        bytes: usize,
    ) -> Result<(), ConstantFoldError> {
        *held += bytes;
        self.retain_memory_bytes(bytes, self.span)
    }

    pub(super) fn reserve_scratch<T>(
        &mut self,
        buffer: &mut Vec<T>,
        additional: usize,
        held: &mut usize,
    ) -> Result<(), ConstantFoldError> {
        let required = buffer.len() + additional;
        let capacity = buffer.capacity();
        if required > capacity {
            let target = required.max(4).next_power_of_two();
            self.charge_scratch(held, (target - capacity) * std::mem::size_of::<T>())?;
            buffer.reserve_exact(target - buffer.len());
        }
        Ok(())
    }

    pub(super) fn with_scratch<T>(
        &mut self,
        run: impl FnOnce(&mut Self, &mut usize) -> Result<T, ConstantFoldError>,
    ) -> Result<T, ConstantFoldError> {
        let mut held = 0;
        let result = run(self, &mut held);
        self.release_memory_bytes(held);
        result
    }

    pub(super) fn release_memory_bytes(&mut self, bytes: usize) {
        if bytes != 0 {
            self.retained_memory_bytes -= bytes;
            self.meter.borrow_mut().retained_memory_bytes -= bytes;
        }
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
    charge_value_range(
        constants,
        string_table,
        start,
        constants.value_count(),
        budget,
    )
}

/// Extend the tracked range over a value whose allocation was checked before insertion.
pub(super) fn register_precharged_value(
    constants: &ConstValueStore,
    string_table: &StringTable,
    index: usize,
    budget: &mut ExecutionBudget,
) -> Result<(), ConstantFoldError> {
    charge_value_range(constants, string_table, index, index, budget)?;
    if let Some((_, end)) = &mut budget.charged_values {
        *end = (*end).max(index + 1);
    }
    Ok(())
}

fn charge_value_range(
    constants: &ConstValueStore,
    string_table: &StringTable,
    start: usize,
    end: usize,
    budget: &mut ExecutionBudget,
) -> Result<(), ConstantFoldError> {
    let (mut from, mut to) = budget.charged_values.unwrap_or((start, start));
    if start < from {
        charge_value_rows(constants, string_table, start, from, budget)?;
        from = start;
    }
    if end > to {
        charge_value_rows(constants, string_table, to, end, budget)?;
        to = end;
    }
    budget.charged_values = Some((from, to));
    Ok(())
}

fn charge_value_rows(
    constants: &ConstValueStore,
    string_table: &StringTable,
    start: usize,
    end: usize,
    budget: &mut ExecutionBudget,
) -> Result<(), ConstantFoldError> {
    for index in start..end {
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
    let Some(metadata) = constants.metadata(value) else {
        return (0, 0);
    };
    let row_bytes = std::mem::size_of::<ConstValue>()
        .saturating_add(metadata.synthetic_interface_provenance.owned_allocation_bytes());
    let Some(payload) = constants.owned_payload(value) else {
        return (row_bytes, 1);
    };
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
