//! Terminator-level borrow transfer rules.
//!
//! WHAT: applies borrow effects for branch, match, and return terminators.
//! WHY: terminators join control-flow analysis with expression access checks, so their
//! transfer entrypoint is kept separate from ordinary statement handling.

use super::*;

pub(crate) fn transfer_terminator(
    context: &BorrowTransferContext<'_>,
    layout: &FunctionLayout,
    state: &mut BorrowState,
    block_id: BlockId,
    terminator: &HirTerminator,
    stats: &mut BlockTransferStats,
    value_fact_buffer: &mut ValueFactBuffer,
) -> Result<(), BorrowCheckError> {
    let mut tracker = StatementAccessTracker::new(layout.local_count());
    // Forward the side-table authored terminator span; generated terminators stay spanless.
    let location = context.diagnostics.terminator_error_span(block_id);
    let conflicts_before = stats.conflicts_checked;
    let terminator_order = layout.terminator_order_or_unknown(block_id);
    let mut input = super::statement::CallTransferContext::new(
        context,
        OperationView::from_layout(layout),
        state,
        block_id,
        terminator_order,
        location,
        stats,
        Some(ValidationSink {
            layout,
            tracker: &mut tracker,
            value_facts: value_fact_buffer,
        }),
    );
    transfer_terminator_operation(&mut input, context, terminator)?;

    stats.terminator_fact = Some((
        block_id,
        TerminatorBorrowFact {
            shared_roots: roots_to_local_ids(layout, &tracker.shared_roots),
            mutable_roots: roots_to_local_ids(layout, &tracker.mutable_roots),
            conflicts_checked: stats.conflicts_checked - conflicts_before,
        },
    ));

    Ok(())
}

pub(in super::super) fn transfer_terminator_operation(
    input: &mut super::statement::CallTransferContext<'_, '_, '_, '_, '_, '_>,
    context: &BorrowTransferContext<'_>,
    terminator: &HirTerminator,
) -> Result<(), BorrowCheckError> {
    use super::statement::{read_statement_operands, transfer_invoke, transfer_never_call};

    match terminator {
        HirTerminator::Jump { .. }
        | HirTerminator::RuntimeFailure { .. }
        | HirTerminator::Break { .. }
        | HirTerminator::Continue { .. } => {}
        HirTerminator::If { condition, .. } => read_statement_operands(input, [*condition])?,
        HirTerminator::Invoke { .. } => transfer_invoke(input, terminator)?,
        HirTerminator::Match { scrutinee, arms } => {
            read_statement_operands(input, [*scrutinee])?;
            for arm in arms {
                input.read_match_pattern(arm)?;
            }
        }
        HirTerminator::Return(values) => {
            read_statement_operands(input, context.expressions.values(*values).iter().copied())?;
        }
        HirTerminator::ReturnError(value) | HirTerminator::AssertFailure { message: value, .. } => {
            read_statement_operands(input, [*value])?;
        }
        HirTerminator::Uninitialized => {
            return Err(context
                .diagnostics
                .internal_error("Uninitialized terminator reached borrow validation"));
        }
        HirTerminator::NeverCall { target, args } => {
            transfer_never_call(input, target, context.expressions.values(*args))?;
        }
    }
    Ok(())
}
